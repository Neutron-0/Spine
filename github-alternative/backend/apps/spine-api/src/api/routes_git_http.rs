use super::middleware::AppState;
use crate::core::types::*;
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use spine_core::{HashAlgorithm, ObjectId};
use spine_history::is_ancestor;
use spine_migration::{
    advertise_refs_receive_pack, advertise_refs_upload_pack, format_receive_pack_response,
    handle_upload_pack_request, parse_receive_pack_request, unpack_and_ingest_receive_pack, GitOid,
};
use spine_object_store::CompositeObjectStore;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct InfoRefsQuery {
    pub service: Option<String>,
}

pub fn unauthorized_response() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::WWW_AUTHENTICATE,
        "Basic realm=\"Spine Forge\"".parse().unwrap(),
    );
    (
        StatusCode::UNAUTHORIZED,
        headers,
        "Authentication required for Spine Forge Git transport\n",
    )
        .into_response()
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let mut table = [-1i8; 256];
    for (i, &b) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        .iter()
        .enumerate()
    {
        table[b as usize] = i as i8;
    }
    let clean: Vec<u8> = input
        .bytes()
        .filter(|&b| !b.is_ascii_whitespace() && b != b'=')
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &b in &clean {
        let val = table[b as usize];
        if val < 0 {
            return None;
        }
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

fn parse_basic_auth(encoded: &str) -> Option<(String, String)> {
    let bytes = decode_base64(encoded.trim())?;
    let s = String::from_utf8(bytes).ok()?;
    let (user, pass) = s.split_once(':')?;
    Some((user.to_string(), pass.to_string()))
}

pub async fn authenticate_git_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<crate::core::types::User> {
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    if let Some(auth_val) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        let auth_str = auth_val.trim();
        if let Some(basic_part) = auth_str.strip_prefix("Basic ") {
            if let Some((username, password)) = parse_basic_auth(basic_part) {
                // 1. Password as Personal Access Token (PAT)
                if password.starts_with("sp_pat_") || password.starts_with("spine_pat_") {
                    let p_hash = crate::auth::pat::hash_pat(&password);
                    if let Ok(Some(pat)) = state.db.get_pat_by_hash(&p_hash).await {
                        if !pat.revoked && pat.expires_at_nanos >= now {
                            let _ = state.db.update_pat_last_used(&pat.id, now).await;
                            if let Ok(Some(u)) = state.db.get_user_by_id(&pat.user_id).await {
                                return Some(u);
                            }
                        }
                    }
                }

                // 2. Password as active session token
                let s_hash = crate::auth::session::hash_session_token(&password);
                if let Ok(Some(sess)) = state.db.get_session_by_hash(&s_hash).await {
                    if !sess.revoked && sess.expires_at_nanos >= now {
                        if let Ok(Some(u)) = state.db.get_user_by_id(&sess.user_id).await {
                            return Some(u);
                        }
                    }
                }

                // 3. Username + Argon2id password hash verification
                if let Ok(Some(user)) = state.db.get_user_by_username(&username).await {
                    if let Ok(true) =
                        crate::auth::password::verify_password(&password, &user.password_hash)
                    {
                        return Some(user);
                    }
                }
            }
        } else if let Some(token_str) = auth_str.strip_prefix("Bearer ") {
            let tok = token_str.trim();
            if tok.starts_with("sp_pat_") || tok.starts_with("spine_pat_") {
                let p_hash = crate::auth::pat::hash_pat(tok);
                if let Ok(Some(pat)) = state.db.get_pat_by_hash(&p_hash).await {
                    if !pat.revoked && pat.expires_at_nanos >= now {
                        let _ = state.db.update_pat_last_used(&pat.id, now).await;
                        if let Ok(Some(u)) = state.db.get_user_by_id(&pat.user_id).await {
                            return Some(u);
                        }
                    }
                }
            } else {
                let s_hash = crate::auth::session::hash_session_token(tok);
                if let Ok(Some(sess)) = state.db.get_session_by_hash(&s_hash).await {
                    if !sess.revoked && sess.expires_at_nanos >= now {
                        if let Ok(Some(u)) = state.db.get_user_by_id(&sess.user_id).await {
                            return Some(u);
                        }
                    }
                }
            }
        } else if let Some(remote_token) = auth_str.strip_prefix("RemoteAuth ") {
            let salt = crate::api::routes_lfs::resolve_lfs_secret_salt();
            if let Some(tok) = crate::api::routes_lfs::verify_remote_auth_token(
                &salt,
                remote_token.trim(),
            ) {
                if let Ok(Some(u)) = state.db.get_user_by_id(&tok.user_id).await {
                    return Some(u);
                }
            }
        }
    }

    if let Some(pat_val) = headers.get("X-Spine-Token").and_then(|v| v.to_str().ok()) {
        let p_hash = crate::auth::pat::hash_pat(pat_val.trim());
        if let Ok(Some(pat)) = state.db.get_pat_by_hash(&p_hash).await {
            if !pat.revoked && pat.expires_at_nanos >= now {
                let _ = state.db.update_pat_last_used(&pat.id, now).await;
                if let Ok(Some(u)) = state.db.get_user_by_id(&pat.user_id).await {
                    return Some(u);
                }
            }
        }
    }

    None
}

pub fn ensure_repo_spine_dir(storage_root: &std::path::Path, native_repo_id: &str) -> PathBuf {
    let spine_dir = storage_root
        .join("repos")
        .join(native_repo_id)
        .join(".spine");
    if !spine_dir.exists() {
        let _ = fs::create_dir_all(spine_dir.join("objects").join("pack"));
        let _ = fs::create_dir_all(spine_dir.join("refs").join("heads"));
        let _ = fs::create_dir_all(spine_dir.join("refs").join("tags"));
        let _ = fs::write(spine_dir.join("HEAD"), "ref: refs/heads/main\n");
    }
    spine_dir
}

fn scan_blob_for_secrets(content: &[u8]) -> Option<String> {
    if let Ok(text) = std::str::from_utf8(content) {
        for line in text.lines() {
            if let Some(pos) = line.find("sp_pat_") {
                let candidate: String = line[pos..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if candidate.len() >= 35 {
                    return Some("Spine Personal Access Token".to_string());
                }
            }
            if let Some(pos) = line.find("ghp_") {
                let candidate: String = line[pos..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect();
                if candidate.len() >= 40 {
                    return Some("GitHub Personal Access Token".to_string());
                }
            }
            if let Some(pos) = line.find("AKIA") {
                let candidate: String = line[pos..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect();
                if candidate.len() >= 20 {
                    return Some("AWS Access Key".to_string());
                }
            }
            if line.contains("-----BEGIN PRIVATE KEY-----")
                || line.contains("-----BEGIN RSA PRIVATE KEY-----")
                || line.contains("-----BEGIN OPENSSH PRIVATE KEY-----")
            {
                return Some("Private Key / Keypair".to_string());
            }
        }
    }
    None
}

/// GET `/:owner/:repo_git/info/refs?service=git-upload-pack|git-receive-pack`
pub async fn git_info_refs(
    State(state): State<AppState>,
    Path((owner, repo_git)): Path<(String, String)>,
    Query(query): Query<InfoRefsQuery>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = crate::api::routes_lfs::check_transport_security(&state, &headers) {
        return resp;
    }

    let service = match query.service.as_deref() {
        Some("git-upload-pack") => "git-upload-pack",
        Some("git-receive-pack") => "git-receive-pack",
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "Smart HTTP service parameter (?service=git-upload-pack|git-receive-pack) is required\n",
            )
                .into_response();
        }
    };

    let repo_name = repo_git.strip_suffix(".git").unwrap_or(&repo_git);
    let caller = authenticate_git_user(&state, &headers).await;

    // Resolve repository
    let repo = match state
        .db
        .get_repository_by_owner_name(&owner, repo_name)
        .await
    {
        Ok(Some(r)) => r,
        _ => return (StatusCode::NOT_FOUND, "Repository not found\n").into_response(),
    };

    if service == "git-upload-pack" {
        // Read access check
        if state
            .permissions
            .check_read_access(caller.as_ref(), &repo)
            .await
            .is_err()
        {
            if caller.is_none() {
                return unauthorized_response();
            }
            return (StatusCode::NOT_FOUND, "Repository not found\n").into_response();
        }

        let spine_dir = ensure_repo_spine_dir(state.vcs.storage_root(), &repo.native_repo_id);
        match advertise_refs_upload_pack(&spine_dir) {
            Ok(body) => {
                let mut resp_headers = HeaderMap::new();
                resp_headers.insert(
                    header::CONTENT_TYPE,
                    format!("application/x-{}-advertisement", service)
                        .parse()
                        .unwrap(),
                );
                resp_headers.insert(
                    header::CACHE_CONTROL,
                    "no-cache, max-age=0, must-revalidate".parse().unwrap(),
                );
                resp_headers.insert(header::PRAGMA, "no-cache".parse().unwrap());
                resp_headers.insert(
                    header::EXPIRES,
                    "Fri, 01 Jan 1980 00:00:00 GMT".parse().unwrap(),
                );
                (StatusCode::OK, resp_headers, body).into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to advertise upload-pack refs: {}\n", e),
            )
                .into_response(),
        }
    } else {
        // git-receive-pack: Write access requires authentication
        let user = match caller {
            Some(u) => u,
            None => return unauthorized_response(),
        };

        if let Err(e) = state
            .permissions
            .check_write_access(Some(&user), &repo)
            .await
        {
            return (
                StatusCode::FORBIDDEN,
                format!("Write permission denied: {}\n", e),
            )
                .into_response();
        }

        let spine_dir = ensure_repo_spine_dir(state.vcs.storage_root(), &repo.native_repo_id);
        match advertise_refs_receive_pack(&spine_dir) {
            Ok(body) => {
                let mut resp_headers = HeaderMap::new();
                resp_headers.insert(
                    header::CONTENT_TYPE,
                    format!("application/x-{}-advertisement", service)
                        .parse()
                        .unwrap(),
                );
                resp_headers.insert(
                    header::CACHE_CONTROL,
                    "no-cache, max-age=0, must-revalidate".parse().unwrap(),
                );
                resp_headers.insert(header::PRAGMA, "no-cache".parse().unwrap());
                resp_headers.insert(
                    header::EXPIRES,
                    "Fri, 01 Jan 1980 00:00:00 GMT".parse().unwrap(),
                );
                (StatusCode::OK, resp_headers, body).into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to advertise receive-pack refs: {}\n", e),
            )
                .into_response(),
        }
    }
}

/// POST `/:owner/:repo_git/git-upload-pack`
pub async fn git_upload_pack(
    State(state): State<AppState>,
    Path((owner, repo_git)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = crate::api::routes_lfs::check_transport_security(&state, &headers) {
        return resp;
    }

    let repo_name = repo_git.strip_suffix(".git").unwrap_or(&repo_git);
    let caller = authenticate_git_user(&state, &headers).await;

    let repo = match state
        .db
        .get_repository_by_owner_name(&owner, repo_name)
        .await
    {
        Ok(Some(r)) => r,
        _ => return (StatusCode::NOT_FOUND, "Repository not found\n").into_response(),
    };

    if state
        .permissions
        .check_read_access(caller.as_ref(), &repo)
        .await
        .is_err()
    {
        if caller.is_none() {
            return unauthorized_response();
        }
        return (StatusCode::NOT_FOUND, "Repository not found\n").into_response();
    }

    let spine_dir = ensure_repo_spine_dir(state.vcs.storage_root(), &repo.native_repo_id);
    match handle_upload_pack_request(&spine_dir, &body) {
        Ok(pack_res) => {
            let mut resp_headers = HeaderMap::new();
            resp_headers.insert(
                header::CONTENT_TYPE,
                "application/x-git-upload-pack-result".parse().unwrap(),
            );
            resp_headers.insert(
                header::CACHE_CONTROL,
                "no-cache, max-age=0, must-revalidate".parse().unwrap(),
            );
            (StatusCode::OK, resp_headers, pack_res).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Upload pack error: {}\n", e),
        )
            .into_response(),
    }
}

/// POST `/:owner/:repo_git/git-receive-pack`
pub async fn git_receive_pack(
    State(state): State<AppState>,
    Path((owner, repo_git)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let _write_permit = match state.consistency_barrier.acquire_write_permit().await {
        Ok(permit) => permit,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(axum::http::header::RETRY_AFTER, "5")],
                "Maintenance in progress. Backups active, repository writes temporarily drained.\n",
            )
                .into_response();
        }
    };

    if let Err(resp) = crate::api::routes_lfs::check_transport_security(&state, &headers) {
        return resp;
    }

    let repo_name = repo_git.strip_suffix(".git").unwrap_or(&repo_git);
    let user = match authenticate_git_user(&state, &headers).await {
        Some(u) => u,
        None => return unauthorized_response(),
    };

    let repo = match state
        .db
        .get_repository_by_owner_name(&owner, repo_name)
        .await
    {
        Ok(Some(r)) => r,
        _ => return (StatusCode::NOT_FOUND, "Repository not found\n").into_response(),
    };

    if let Err(e) = state
        .permissions
        .check_write_access(Some(&user), &repo)
        .await
    {
        return (
            StatusCode::FORBIDDEN,
            format!("Write permission denied: {}\n", e),
        )
            .into_response();
    }

    let resp_bytes = match process_git_receive_pack(&state, &repo, &user.id, &body).await {
        Ok(b) => b,
        Err((code, msg)) => return (code, msg).into_response(),
    };

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        header::CONTENT_TYPE,
        "application/x-git-receive-pack-result".parse().unwrap(),
    );
    resp_headers.insert(
        header::CACHE_CONTROL,
        "no-cache, max-age=0, must-revalidate".parse().unwrap(),
    );

    (StatusCode::OK, resp_headers, resp_bytes).into_response()
}

pub fn flatten_tree_recursive<S: spine_object_store::ObjectStore>(
    store: &S,
    tree_id: &ObjectId,
    prefix: &str,
    out: &mut std::collections::BTreeMap<String, (spine_core::FileMode, ObjectId)>,
) -> anyhow::Result<()> {
    let mut active_ancestors = std::collections::HashSet::new();
    flatten_tree_recursive_inner(store, tree_id, prefix, out, &mut active_ancestors, 0)
}

fn flatten_tree_recursive_inner<S: spine_object_store::ObjectStore>(
    store: &S,
    tree_id: &ObjectId,
    prefix: &str,
    out: &mut std::collections::BTreeMap<String, (spine_core::FileMode, ObjectId)>,
    active_ancestors: &mut std::collections::HashSet<ObjectId>,
    depth: usize,
) -> anyhow::Result<()> {
    const MAX_TREE_DEPTH: usize = 128;
    if depth > MAX_TREE_DEPTH {
        anyhow::bail!("Tree nesting exceeds maximum allowable depth ({})", MAX_TREE_DEPTH);
    }
    if !active_ancestors.insert(*tree_id) {
        anyhow::bail!("Cyclic tree reference detected at object {}", tree_id);
    }

    let res = (|| -> anyhow::Result<()> {
        let (obj_type, payload) = store.read_object(tree_id)?;
        if obj_type != spine_core::ObjectType::Tree {
            return Ok(());
        }
        let entries = spine_format::deserialize_tree(&payload)?;
        for entry in entries {
            let full_path = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{}/{}", prefix, entry.name)
            };
            if entry.mode == spine_core::FileMode::Directory {
                flatten_tree_recursive_inner(store, &entry.object_id, &full_path, out, active_ancestors, depth + 1)?;
            } else {
                out.insert(full_path, (entry.mode, entry.object_id));
            }
        }
        Ok(())
    })();

    active_ancestors.remove(tree_id);
    res
}

pub fn collect_changed_paths_from_commits<S: spine_object_store::ObjectStore>(
    store: &S,
    new_commits: &[(GitOid, ObjectId, spine_core::Commit)],
) -> anyhow::Result<Vec<String>> {
    let mut paths = std::collections::HashSet::new();
    for (_goid, _sid, commit) in new_commits {
        let mut commit_tree_files = std::collections::BTreeMap::new();
        flatten_tree_recursive(store, &commit.root_tree, "", &mut commit_tree_files)?;

        if commit.parents.is_empty() {
            for path in commit_tree_files.keys() {
                paths.insert(path.clone());
            }
        } else {
            for parent_id in &commit.parents {
                let (obj_type, payload) = store.read_object(parent_id)?;
                if obj_type == spine_core::ObjectType::Commit {
                    let parent_commit = spine_format::deserialize_commit(&payload)?;
                    let mut parent_tree_files = std::collections::BTreeMap::new();
                    flatten_tree_recursive(
                        store,
                        &parent_commit.root_tree,
                        "",
                        &mut parent_tree_files,
                    )?;
                    let diff =
                        spine_diff::compare_trees(&parent_tree_files, &commit_tree_files);
                    for change in diff {
                        paths.insert(change.path().to_string());
                    }
                }
            }
        }
    }
    Ok(paths.into_iter().collect())
}

pub async fn process_git_receive_pack(
    state: &AppState,
    repo: &ForgeRepository,
    actor_user_id: &str,
    body: &[u8],
) -> Result<Vec<u8>, (StatusCode, String)> {
    let spine_dir = ensure_repo_spine_dir(state.vcs.storage_root(), &repo.native_repo_id);

    // Parse commands and packfile payload
    let (commands, _capabilities, pack_bytes) = match parse_receive_pack_request(body) {
        Ok(res) => res,
        Err(e) => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Failed to parse receive-pack request: {}\n", e),
            ));
        }
    };

    if commands.is_empty() {
        return Ok(spine_migration::encode_flush().to_vec());
    }

    // Ingest packfile into native Spine repository
    let ingest_res = match unpack_and_ingest_receive_pack(&spine_dir, &pack_bytes) {
        Ok(res) => res,
        Err(e) => {
            let err_str = e.to_string();
            let resp_bytes = format_receive_pack_response(false, Some(&err_str), &[])
                .unwrap_or_else(|_| spine_migration::encode_flush().to_vec());
            return Ok(resp_bytes);
        }
    };

    // 1. Security Check: Secret Scanning on new blobs
    let mut detected_secret: Option<String> = None;
    for (_oid, payload) in &ingest_res.new_blobs {
        if let Some(secret_name) = scan_blob_for_secrets(payload) {
            detected_secret = Some(secret_name);
            break;
        }
    }

    // 1b. LFS Object Presence Verification: Check newly introduced LFS pointer blobs
    let mut detected_missing_lfs: Option<(String, u64)> = None;
    for (_oid, payload) in &ingest_res.new_blobs {
        if spine_migration::GitLfsPointer::is_lfs_pointer(payload) {
            if let Ok(ptr) = spine_migration::GitLfsPointer::parse(payload) {
                let exists = state
                    .db
                    .get_artifact_manifest(&ptr.oid_sha256)
                    .await
                    .map(|opt| opt.is_some())
                    .unwrap_or(false);
                if !exists {
                    detected_missing_lfs = Some((ptr.oid_sha256, ptr.size_bytes));
                    break;
                }
            }
        }
    }

    let protection_rules = state
        .db
        .list_branch_protection_rules(&repo.id)
        .await
        .unwrap_or_default();

    let store = match CompositeObjectStore::new(&spine_dir, HashAlgorithm::Blake3, 3) {
        Ok(s) => s,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to open object store: {}\n", e),
            ));
        }
    };

    // 1c. LFS Lock Conflict Verification: Check modified paths in incoming commits
    let changed_paths = match collect_changed_paths_from_commits(&store, &ingest_res.new_commits) {
        Ok(paths) => paths,
        Err(e) => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Failed to verify changed commit trees during lock check: {}\n", e),
            ));
        }
    };
    let mut detected_lock_conflict: Option<(String, String)> = None;
    for path in &changed_paths {
        if let Ok(Some(lock)) = state.db.get_lfs_lock_by_path(&repo.id, path).await {
            if lock.owner_id != actor_user_id {
                let _ = state
                    .db
                    .create_lfs_lock_audit(LfsLockAudit {
                        id: uuid::Uuid::new_v4().to_string(),
                        repository_id: repo.id.clone(),
                        path: path.clone(),
                        actor_user_id: actor_user_id.to_string(),
                        action: "CONFLICT".to_string(),
                        created_at_nanos: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
                    })
                    .await;
                detected_lock_conflict = Some((path.clone(), lock.owner_username.clone()));
                break;
            }
        }
    }

    let mut ref_results: Vec<(String, Result<(), String>)> = Vec::new();

    for cmd in &commands {
        let ref_name = &cmd.ref_name;

        // If secret was detected, reject all ref updates
        if let Some(ref sec) = detected_secret {
            ref_results.push((
                ref_name.clone(),
                Err(format!(
                    "push rejected: secret scanning detected sensitive credentials ({})",
                    sec
                )),
            ));
            continue;
        }

        // If referenced LFS object is missing, reject all ref updates
        if let Some((ref lfs_oid, lfs_sz)) = detected_missing_lfs {
            ref_results.push((
                ref_name.clone(),
                Err(format!(
                    "push rejected: referenced LFS object '{}' ({} bytes) is missing; upload via git-lfs before pushing",
                    lfs_oid, lfs_sz
                )),
            ));
            continue;
        }

        // If LFS lock conflict was detected, reject all ref updates
        if let Some((ref locked_path, ref lock_owner)) = detected_lock_conflict {
            ref_results.push((
                ref_name.clone(),
                Err(format!(
                    "push rejected: file '{}' is locked by {}",
                    locked_path, lock_owner
                )),
            ));
            continue;
        }

        // Branch protection validation
        let branch_name = ref_name.strip_prefix("refs/heads/").unwrap_or(ref_name);
        let rule = protection_rules
            .iter()
            .find(|r| r.branch_pattern == branch_name || r.branch_pattern == "*");

        if let Some(r) = rule {
            if r.require_pull_request && cmd.new_oid != GitOid::zero() {
                ref_results.push((
                    ref_name.clone(),
                    Err(format!(
                        "push rejected: direct push to protected branch '{}' is prohibited; pull request required",
                        branch_name
                    )),
                ));
                continue;
            }
        }

        // CAS & Ref Update Resolution
        let rel_path = ref_name.strip_prefix("refs/").unwrap_or(ref_name);
        let ref_file = spine_dir.join("refs").join(rel_path);

        let current_spine_id = if ref_file.exists() {
            fs::read_to_string(&ref_file)
                .ok()
                .and_then(|c| ObjectId::from_hex(c.trim()).ok())
        } else {
            None
        };

        // CAS Validation against cmd.old_oid
        if cmd.old_oid != GitOid::zero() {
            let current_git_oid =
                current_spine_id.and_then(|sid| ingest_res.spine_to_git.get(&sid).copied());
            if let Some(cur_git) = current_git_oid {
                if cur_git != cmd.old_oid {
                    ref_results.push((
                        ref_name.clone(),
                        Err(format!(
                            "push rejected: stale reference (CAS mismatch: current {}, expected {})",
                            cur_git, cmd.old_oid
                        )),
                    ));
                    continue;
                }
            } else {
                ref_results.push((
                    ref_name.clone(),
                    Err(format!(
                        "push rejected: stale reference (CAS mismatch: current_spine_id {:?} not found in spine_to_git, expected {})",
                        current_spine_id, cmd.old_oid
                    )),
                ));
                continue;
            }
        }

        // Fast-Forward check for non-deletion branch updates on protected branches
        if let (Some(cur_sid), Some(_r)) = (current_spine_id, rule) {
            if cmd.new_oid != GitOid::zero() {
                if let Some(&new_sid) = ingest_res.git_to_spine.get(&cmd.new_oid) {
                    let is_ff = is_ancestor(&store, &cur_sid, &new_sid).unwrap_or(false);
                    if !is_ff {
                        ref_results.push((
                            ref_name.clone(),
                            Err(format!(
                                "push rejected: force push to protected branch '{}' is strictly prohibited",
                                branch_name
                            )),
                        ));
                        continue;
                    }
                }
            }
        }

        // Apply update
        if cmd.new_oid == GitOid::zero() {
            // Delete ref
            if ref_file.exists() {
                let _ = fs::remove_file(&ref_file);
            }
            ref_results.push((ref_name.clone(), Ok(())));
        } else {
            // Create or update ref
            let new_spine_id = match ingest_res.git_to_spine.get(&cmd.new_oid) {
                Some(id) => *id,
                None => {
                    ref_results.push((
                        ref_name.clone(),
                        Err("push rejected: commit object not found in pack".to_string()),
                    ));
                    continue;
                }
            };

            if let Some(parent) = ref_file.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Err(e) = fs::write(&ref_file, format!("{}\n", new_spine_id.to_hex())) {
                ref_results.push((
                    ref_name.clone(),
                    Err(format!("push rejected: failed to write reference: {}", e)),
                ));
                continue;
            }

            // Ensure HEAD is updated if updating main branch or default branch
            if branch_name == repo.default_branch
                || branch_name == "main"
                || branch_name == "master"
            {
                let head_file = spine_dir.join("HEAD");
                let symref_target = format!("ref: refs/heads/{}\n", branch_name);
                let _ = fs::write(head_file, symref_target);
            }

            ref_results.push((ref_name.clone(), Ok(())));
        }
    }

    let resp_bytes = format_receive_pack_response(true, None, &ref_results).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to format receive-pack response: {}\n", e),
        )
    })?;

    Ok(resp_bytes)
}
