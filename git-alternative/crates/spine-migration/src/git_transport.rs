use crate::git_exporter::{ExportStats, GitExporter};
use crate::git_format::{
    build_pack_and_index, parse_git_commit, parse_git_tag, parse_git_tree,
    parse_packfile_with_resolver, read_git_refs, GitHead, GitObjectType, GitOid, GitRefMap,
    PackObjectToBuild,
};
use crate::pkt_line::{
    encode_flush, encode_pkt_line, encode_pkt_line_str, encode_sideband, parse_pkt_line,
    PktLine, PktLineError,
};
use spine_core::{AnnotatedTag, Commit, FileMode, HashAlgorithm, ObjectId, ObjectType, TreeEntry};
use spine_format::{
    deserialize_annotated_tag, deserialize_commit, deserialize_tree, serialize_annotated_tag,
    serialize_commit, serialize_tree,
};
use spine_object_store::{CompositeObjectStore, FileSystemObjectStore, ObjectStore};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitTransportError {
    #[error("I/O error during Git transport: {0}")]
    Io(#[from] io::Error),
    #[error("Packet-line codec error: {0}")]
    PktLine(#[from] PktLineError),
    #[error("Pack parser error: {0}")]
    PackParse(#[from] crate::git_format::PackParseError),
    #[error("Pack builder error: {0}")]
    PackBuild(#[from] crate::git_format::PackBuildError),
    #[error("Spine object store error: {0}")]
    ObjectStore(#[from] spine_object_store::ObjectStoreError),
    #[error("Spine format error: {0}")]
    Format(#[from] spine_format::FormatError),
    #[error("Protocol error: {0}")]
    Protocol(String),
    #[error("Unresolved Git object: {0}")]
    UnresolvedOid(GitOid),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUpdateCommand {
    pub old_oid: GitOid,
    pub new_oid: GitOid,
    pub ref_name: String,
}

#[derive(Debug, Clone)]
pub struct IngestPackResult {
    pub git_to_spine: HashMap<GitOid, ObjectId>,
    pub spine_to_git: HashMap<ObjectId, GitOid>,
    pub new_blobs: Vec<(GitOid, Vec<u8>)>,
    pub new_commits: Vec<(GitOid, ObjectId, Commit)>,
}

/// Generates the reference advertisement payload for `GET /info/refs?service=git-upload-pack`.
pub fn advertise_refs_upload_pack(spine_dir: &Path) -> Result<Vec<u8>, GitTransportError> {
    let mut out = Vec::new();

    // 1. Service announcement packet
    out.extend_from_slice(&encode_pkt_line_str("# service=git-upload-pack\n")?);
    out.extend_from_slice(encode_flush());

    // 2. Discover refs and convert to Git OIDs
    let (ref_map, default_branch) = resolve_repository_git_refs(spine_dir)?;

    let capabilities = format!(
        "symref=HEAD:refs/heads/{} multi_ack_detailed no-done side-band-64k ofs-delta agent=spine/0.1.0",
        default_branch
    );

    if ref_map.branches.is_empty() && ref_map.tags.is_empty() {
        // Empty repository advertisement
        let empty_line = format!(
            "0000000000000000000000000000000000000000 capabilities^{{}}\0{}\n",
            capabilities
        );
        out.extend_from_slice(&encode_pkt_line(empty_line.as_bytes())?);
        out.extend_from_slice(encode_flush());
        return Ok(out);
    }

    // 3. First line MUST advertise HEAD (or first ref) with capabilities separated by NUL \0
    let head_oid = match ref_map.head {
        Some(GitHead::Detached(oid)) => Some(oid),
        Some(GitHead::Symbolic(ref target)) => {
            if let Some(bname) = target.strip_prefix("refs/heads/") {
                ref_map.branches.get(bname).copied()
            } else {
                None
            }
        }
        None => None,
    }
    .or_else(|| ref_map.branches.get(&default_branch).copied())
    .or_else(|| ref_map.branches.values().next().copied())
    .unwrap_or(GitOid::from_bytes([0u8; 20]));

    let head_line = format!("{} HEAD\0{}\n", head_oid, capabilities);
    out.extend_from_slice(&encode_pkt_line(head_line.as_bytes())?);

    // 4. Remaining branches and tags sorted lexicographically
    let mut sorted_refs: Vec<(String, GitOid, Option<GitOid>)> = Vec::new();
    for (branch, &oid) in &ref_map.branches {
        sorted_refs.push((format!("refs/heads/{}", branch), oid, None));
    }
    for (tag, &oid) in &ref_map.tags {
        sorted_refs.push((
            format!("refs/tags/{}", tag),
            oid,
            ref_map.peeled_tags.get(tag).copied(),
        ));
    }
    sorted_refs.sort_by(|a, b| a.0.cmp(&b.0));

    for (ref_name, oid, peeled) in sorted_refs {
        let line = format!("{} {}\n", oid, ref_name);
        out.extend_from_slice(&encode_pkt_line(line.as_bytes())?);
        if let Some(peeled_oid) = peeled {
            let peeled_line = format!("{} {}^{{}}\n", peeled_oid, ref_name);
            out.extend_from_slice(&encode_pkt_line(peeled_line.as_bytes())?);
        }
    }

    out.extend_from_slice(encode_flush());
    Ok(out)
}

/// Generates the reference advertisement payload for `GET /info/refs?service=git-receive-pack`.
pub fn advertise_refs_receive_pack(spine_dir: &Path) -> Result<Vec<u8>, GitTransportError> {
    let mut out = Vec::new();

    // 1. Service announcement packet
    out.extend_from_slice(&encode_pkt_line_str("# service=git-receive-pack\n")?);
    out.extend_from_slice(encode_flush());

    // 2. Discover refs and convert to Git OIDs
    let (ref_map, _) = resolve_repository_git_refs(spine_dir)?;
    let capabilities = "report-status delete-refs ofs-delta agent=spine/0.1.0";

    if ref_map.branches.is_empty() && ref_map.tags.is_empty() {
        // Empty repository advertisement
        let empty_line = format!(
            "0000000000000000000000000000000000000000 capabilities^{{}}\0{}\n",
            capabilities
        );
        out.extend_from_slice(&encode_pkt_line(empty_line.as_bytes())?);
        out.extend_from_slice(encode_flush());
        return Ok(out);
    }

    // Sort all refs lexicographically
    let mut sorted_refs: Vec<(String, GitOid)> = Vec::new();
    for (branch, &oid) in &ref_map.branches {
        sorted_refs.push((format!("refs/heads/{}", branch), oid));
    }
    for (tag, &oid) in &ref_map.tags {
        sorted_refs.push((format!("refs/tags/{}", tag), oid));
    }
    sorted_refs.sort_by(|a, b| a.0.cmp(&b.0));

    // First line includes capabilities
    let (first_ref, first_oid) = &sorted_refs[0];
    let first_line = format!("{} {}\0{}\n", first_oid, first_ref, capabilities);
    out.extend_from_slice(&encode_pkt_line(first_line.as_bytes())?);

    // Remaining lines
    for (ref_name, oid) in &sorted_refs[1..] {
        let line = format!("{} {}\n", oid, ref_name);
        out.extend_from_slice(&encode_pkt_line(line.as_bytes())?);
    }

    out.extend_from_slice(encode_flush());
    Ok(out)
}

/// Processes a `POST /git-upload-pack` request, builds the requested Git packfile,
/// and returns the framed response.
pub fn handle_upload_pack_request(
    spine_dir: &Path,
    request_body: &[u8],
) -> Result<Vec<u8>, GitTransportError> {
    let mut wants = Vec::new();
    let mut capabilities = Vec::new();
    let mut _haves = Vec::new();

    let mut cursor = 0;
    while let Some((pkt, consumed)) = parse_pkt_line(&request_body[cursor..])? {
        cursor += consumed;
        match pkt {
            PktLine::Data(bytes) => {
                let line = String::from_utf8_lossy(&bytes);
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("want ") {
                    let parts: Vec<&str> = rest.split_whitespace().collect();
                    if let Some(oid_str) = parts.first() {
                        if let Ok(oid) = GitOid::from_hex(oid_str) {
                            wants.push(oid);
                        }
                    }
                    if capabilities.is_empty() && parts.len() > 1 {
                        for cap in &parts[1..] {
                            capabilities.push(cap.to_string());
                        }
                    }
                } else if let Some(rest) = trimmed.strip_prefix("have ") {
                    if let Ok(oid) = GitOid::from_hex(rest.trim()) {
                        _haves.push(oid);
                    }
                }
            }
            PktLine::Flush => break,
            _ => {}
        }
    }

    if wants.is_empty() {
        return Ok(encode_flush().to_vec());
    }

    let use_sideband = capabilities
        .iter()
        .any(|c| c == "side-band-64k" || c == "side-band");

    // Build the packfile containing all objects needed for wants
    let pack_bytes = generate_pack_for_wants(spine_dir, &wants)?;

    let mut response = Vec::new();
    // 1. NAK packet indicating object negotiation concluded
    response.extend_from_slice(&encode_pkt_line_str("NAK\n")?);

    // 2. Transmit packfile payload
    if use_sideband {
        // Transmit packfile on sideband channel 1 (data)
        response.extend_from_slice(&encode_sideband(1, &pack_bytes));
        response.extend_from_slice(encode_flush());
    } else {
        response.extend_from_slice(&pack_bytes);
    }

    Ok(response)
}

/// Parses the commands and packfile payload from a `POST /git-receive-pack` request.
pub fn parse_receive_pack_request(
    request_body: &[u8],
) -> Result<(Vec<RefUpdateCommand>, Vec<String>, Vec<u8>), GitTransportError> {
    let mut commands = Vec::new();
    let mut capabilities = Vec::new();
    let mut cursor = 0;

    while let Some((pkt, consumed)) = parse_pkt_line(&request_body[cursor..])? {
        cursor += consumed;
        match pkt {
            PktLine::Data(bytes) => {
                let s = String::from_utf8_lossy(&bytes);
                let trimmed = s.trim();

                let (cmd_part, caps_part) = if let Some((c, caps)) = trimmed.split_once('\0') {
                    (c, Some(caps))
                } else {
                    (trimmed, None)
                };

                if let Some(caps) = caps_part {
                    if capabilities.is_empty() {
                        for cap in caps.split_whitespace() {
                            capabilities.push(cap.to_string());
                        }
                    }
                }

                let parts: Vec<&str> = cmd_part.split_whitespace().collect();
                if parts.len() >= 3 {
                    if let (Ok(old_oid), Ok(new_oid)) =
                        (GitOid::from_hex(parts[0]), GitOid::from_hex(parts[1]))
                    {
                        commands.push(RefUpdateCommand {
                            old_oid,
                            new_oid,
                            ref_name: parts[2].to_string(),
                        });
                    }
                }
            }
            PktLine::Flush => break,
            _ => {}
        }
    }

    let pack_bytes = if cursor < request_body.len() {
        request_body[cursor..].to_vec()
    } else {
        Vec::new()
    };

    Ok((commands, capabilities, pack_bytes))
}

/// Ingests objects from a received Git packfile directly into the native Spine repository.
pub fn unpack_and_ingest_receive_pack(
    spine_dir: &Path,
    pack_bytes: &[u8],
) -> Result<IngestPackResult, GitTransportError> {
    let store = FileSystemObjectStore::new(spine_dir, HashAlgorithm::Blake3, 3)?;

    let (persisted_git_to_spine, persisted_spine_to_git) = load_git_map(spine_dir);
    let mut git_to_spine: HashMap<GitOid, ObjectId> = persisted_git_to_spine;
    let mut spine_to_git: HashMap<ObjectId, GitOid> = persisted_spine_to_git;
    let existing_git_objects: HashMap<GitOid, (GitObjectType, Vec<u8>)> = HashMap::new();

    // Pre-seed mappings from existing repository references
    if let Ok((ref_map, _)) = resolve_repository_git_refs(spine_dir) {
        let tmp_git = spine_dir.join("tmp_git_receive_preseed");
        if let Ok(mut exporter) = GitExporter::new(spine_dir, &tmp_git, true) {
            let mut stats = ExportStats::default();
            for branch in ref_map.branches.keys() {
                let branch_file = spine_dir.join("refs").join("heads").join(branch);
                if let Ok(content) = fs::read_to_string(branch_file) {
                    if let Ok(spine_id) = ObjectId::from_hex(content.trim()) {
                        if !spine_to_git.contains_key(&spine_id) {
                            let _ = exporter.export_commit_recursive_pub(&store, &spine_id, &mut stats);
                        }
                    }
                }
            }
            for tag in ref_map.tags.keys() {
                let tag_file = spine_dir.join("refs").join("tags").join(tag);
                if let Ok(content) = fs::read_to_string(tag_file) {
                    if let Ok(spine_id) = ObjectId::from_hex(content.trim()) {
                        if !spine_to_git.contains_key(&spine_id) {
                            let _ = exporter.export_tag_or_target_pub(&store, &spine_id, &mut stats);
                        }
                    }
                }
            }
            for (sid, gid) in exporter.spine_to_git_map() {
                spine_to_git.entry(*sid).or_insert(*gid);
            }
            for (gid, sid) in exporter.git_to_spine_map() {
                git_to_spine.entry(*gid).or_insert(*sid);
            }
            let _ = fs::remove_dir_all(&tmp_git);
        }
    }

    // Parse the packfile with a resolver that can look up existing Git objects in this repo
    let pack_objects = if !pack_bytes.is_empty() {
        parse_packfile_with_resolver(pack_bytes, |base_oid| {
            if let Some((t, data)) = existing_git_objects.get(base_oid) {
                return Some((*t, data.clone()));
            }
            if let Some(&spine_id) = git_to_spine.get(base_oid) {
                if let Ok((obj_type, payload)) = store.read_object(&spine_id) {
                    let git_type = match obj_type {
                        ObjectType::Blob => GitObjectType::Blob,
                        ObjectType::Tree => GitObjectType::Tree,
                        ObjectType::Commit => GitObjectType::Commit,
                        ObjectType::Tag => GitObjectType::Tag,
                    };
                    return Some((git_type, payload));
                }
            }
            None
        })?
    } else {
        Vec::new()
    };

    let mut raw_objects: HashMap<GitOid, (GitObjectType, Vec<u8>)> = HashMap::new();
    let mut new_blobs = Vec::new();

    for obj in pack_objects {
        save_raw_git_object(spine_dir, &obj.oid, obj.obj_type, &obj.payload);
        raw_objects.insert(obj.oid, (obj.obj_type, obj.payload));
    }

    // 1. Ingest Blobs
    for (&oid, (obj_type, payload)) in &raw_objects {
        if *obj_type == GitObjectType::Blob {
            let spine_id = store.write_object(ObjectType::Blob, payload)?;
            git_to_spine.insert(oid, spine_id);
            spine_to_git.insert(spine_id, oid);
            new_blobs.push((oid, payload.clone()));
        }
    }

    // 2. Ingest Trees
    let tree_oids: Vec<GitOid> = raw_objects
        .iter()
        .filter(|(_, (t, _))| *t == GitObjectType::Tree)
        .map(|(&id, _)| id)
        .collect();

    for oid in tree_oids {
        ingest_tree_recursive(oid, &raw_objects, &mut git_to_spine, &mut spine_to_git, &store)?;
    }

    // 3. Ingest Commits
    let commit_oids: Vec<GitOid> = raw_objects
        .iter()
        .filter(|(_, (t, _))| *t == GitObjectType::Commit)
        .map(|(&id, _)| id)
        .collect();

    let mut new_commits = Vec::new();
    for oid in commit_oids {
        if let Some(res) = ingest_commit_recursive(
            oid,
            &raw_objects,
            &mut git_to_spine,
            &mut spine_to_git,
            &store,
        )? {
            new_commits.push(res);
        }
    }

    // 4. Ingest Tags
    let tag_oids: Vec<GitOid> = raw_objects
        .iter()
        .filter(|(_, (t, _))| *t == GitObjectType::Tag)
        .map(|(&id, _)| id)
        .collect();

    for oid in tag_oids {
        let (_, payload) = raw_objects.get(&oid).unwrap();
        if let Ok(git_tag) = parse_git_tag(payload) {
            if let Some(&target_spine_id) = git_to_spine.get(&git_tag.object) {
                let target_type = match git_tag.target_type.as_str() {
                    "commit" => ObjectType::Commit,
                    "tree" => ObjectType::Tree,
                    "blob" => ObjectType::Blob,
                    _ => ObjectType::Commit,
                };
                let (tagger_name, tagger_email, timestamp) =
                    crate::git_format::commit::parse_identity_line(&git_tag.tagger);

                let spine_tag = AnnotatedTag::new(
                    target_spine_id,
                    target_type,
                    git_tag.tag_name,
                    tagger_name,
                    tagger_email,
                    timestamp * 1_000_000_000,
                    git_tag.message,
                );
                let serialized = serialize_annotated_tag(&spine_tag);
                let spine_id = store.write_object(ObjectType::Tag, &serialized)?;
                git_to_spine.insert(oid, spine_id);
                spine_to_git.insert(spine_id, oid);
            }
        }
    }

    save_git_map(spine_dir, &git_to_spine);

    Ok(IngestPackResult {
        git_to_spine,
        spine_to_git,
        new_blobs,
        new_commits,
    })
}

/// Formats the standard Git `report-status` response for a receive-pack operation.
pub fn format_receive_pack_response(
    unpack_ok: bool,
    unpack_err: Option<&str>,
    ref_results: &[(String, Result<(), String>)],
) -> Result<Vec<u8>, GitTransportError> {
    let mut out = Vec::new();

    if unpack_ok {
        out.extend_from_slice(&encode_pkt_line_str("unpack ok\n")?);
    } else {
        let err_msg = unpack_err.unwrap_or("failed to unpack packfile");
        out.extend_from_slice(&encode_pkt_line_str(&format!("unpack {}\n", err_msg))?);
    }

    for (ref_name, result) in ref_results {
        match result {
            Ok(()) => {
                out.extend_from_slice(&encode_pkt_line_str(&format!("ok {}\n", ref_name))?);
            }
            Err(err) => {
                out.extend_from_slice(&encode_pkt_line_str(&format!("ng {} {}\n", ref_name, err))?);
            }
        }
    }

    out.extend_from_slice(encode_flush());
    Ok(out)
}

fn load_git_map(spine_dir: &Path) -> (HashMap<GitOid, ObjectId>, HashMap<ObjectId, GitOid>) {
    let mut git_to_spine = HashMap::new();
    let mut spine_to_git = HashMap::new();
    let map_file = spine_dir.join("git_map.json");
    if let Ok(data) = fs::read_to_string(map_file) {
        if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&data) {
            for (git_hex, spine_hex) in map {
                if let (Ok(goid), Ok(soid)) = (GitOid::from_hex(&git_hex), ObjectId::from_hex(&spine_hex)) {
                    git_to_spine.insert(goid, soid);
                    spine_to_git.insert(soid, goid);
                }
            }
        }
    }
    (git_to_spine, spine_to_git)
}

fn save_git_map(spine_dir: &Path, git_to_spine: &HashMap<GitOid, ObjectId>) {
    let map_file = spine_dir.join("git_map.json");
    let mut map = HashMap::new();
    for (goid, soid) in git_to_spine {
        map.insert(goid.to_hex(), soid.to_hex());
    }
    if let Ok(json) = serde_json::to_string(&map) {
        let _ = fs::write(map_file, json);
    }
}

fn save_raw_git_object(spine_dir: &Path, oid: &GitOid, obj_type: GitObjectType, payload: &[u8]) {
    let git_objs_dir = spine_dir.join("git_objects");
    let _ = fs::create_dir_all(&git_objs_dir);
    let obj_file = git_objs_dir.join(oid.to_hex());
    let type_byte = match obj_type {
        GitObjectType::Commit => 1u8,
        GitObjectType::Tree => 2u8,
        GitObjectType::Blob => 3u8,
        GitObjectType::Tag => 4u8,
    };
    let mut data = Vec::with_capacity(payload.len() + 1);
    data.push(type_byte);
    data.extend_from_slice(payload);
    let _ = fs::write(obj_file, data);
}

fn load_raw_git_object(spine_dir: &Path, oid: &GitOid) -> Option<(GitObjectType, Vec<u8>)> {
    let obj_file = spine_dir.join("git_objects").join(oid.to_hex());
    let data = fs::read(obj_file).ok()?;
    if data.is_empty() {
        return None;
    }
    let obj_type = match data[0] {
        1 => GitObjectType::Commit,
        2 => GitObjectType::Tree,
        3 => GitObjectType::Blob,
        4 => GitObjectType::Tag,
        _ => return None,
    };
    Some((obj_type, data[1..].to_vec()))
}

fn collect_raw_git_objects_recursive(
    spine_dir: &Path,
    oid: &GitOid,
    pack_objects: &mut Vec<PackObjectToBuild>,
    visited: &mut HashSet<GitOid>,
) -> bool {
    if !visited.insert(*oid) {
        return true;
    }
    if let Some((obj_type, payload)) = load_raw_git_object(spine_dir, oid) {
        match obj_type {
            GitObjectType::Commit => {
                if let Ok(c) = parse_git_commit(&payload) {
                    collect_raw_git_objects_recursive(spine_dir, &c.tree, pack_objects, visited);
                    for p in &c.parents {
                        collect_raw_git_objects_recursive(spine_dir, p, pack_objects, visited);
                    }
                }
            }
            GitObjectType::Tree => {
                if let Ok(entries) = crate::git_format::parse_git_tree(&payload) {
                    for entry in entries {
                        collect_raw_git_objects_recursive(spine_dir, &entry.oid, pack_objects, visited);
                    }
                }
            }
            GitObjectType::Tag => {
                if let Ok(tag) = parse_git_tag(&payload) {
                    collect_raw_git_objects_recursive(spine_dir, &tag.object, pack_objects, visited);
                }
            }
            GitObjectType::Blob => {}
        }
        pack_objects.push(PackObjectToBuild {
            obj_type,
            payload,
            oid: *oid,
        });
        true
    } else {
        false
    }
}

fn resolve_repository_git_refs(
    spine_dir: &Path,
) -> Result<(GitRefMap, String), GitTransportError> {
    let mut ref_map = GitRefMap::default();
    let mut default_branch = "main".to_string();

    let heads_dir = spine_dir.join("refs").join("heads");
    let (_persisted_git_to_spine, mut spine_to_git) = load_git_map(spine_dir);

    if heads_dir.exists() {
        let store = FileSystemObjectStore::new(spine_dir, HashAlgorithm::Blake3, 3)?;
        let mut stats = ExportStats::default();

        let tmp_git = spine_dir.join("tmp_git_refs_export");
        let mut exporter = GitExporter::new(spine_dir, &tmp_git, true)
            .map_err(|e| GitTransportError::Protocol(e.to_string()))?;

        for entry in fs::read_dir(&heads_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                let branch_name = entry.file_name().to_string_lossy().to_string();
                let content = fs::read_to_string(&path)?;
                if let Ok(spine_id) = ObjectId::from_hex(content.trim()) {
                    if let Some(&git_oid) = spine_to_git.get(&spine_id) {
                        ref_map.branches.insert(branch_name, git_oid);
                    } else if let Ok(git_oid) =
                        exporter.export_commit_recursive_pub(&store, &spine_id, &mut stats)
                    {
                        ref_map.branches.insert(branch_name, git_oid);
                        spine_to_git.insert(spine_id, git_oid);
                    }
                }
            }
        }

        let tags_dir = spine_dir.join("refs").join("tags");
        if tags_dir.exists() {
            for entry in fs::read_dir(&tags_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() {
                    let tag_name = entry.file_name().to_string_lossy().to_string();
                    let content = fs::read_to_string(&path)?;
                    if let Ok(spine_id) = ObjectId::from_hex(content.trim()) {
                        if let Some(&git_oid) = spine_to_git.get(&spine_id) {
                            ref_map.tags.insert(tag_name, git_oid);
                        } else if let Ok(git_oid) =
                            exporter.export_tag_or_target_pub(&store, &spine_id, &mut stats)
                        {
                            ref_map.tags.insert(tag_name, git_oid);
                            spine_to_git.insert(spine_id, git_oid);
                        }
                    }
                }
            }
        }
    }

    let head_file = spine_dir.join("HEAD");
    if head_file.exists() {
        let content = fs::read_to_string(head_file)?;
        let trimmed = content.trim();
        if let Some(sym) = trimmed.strip_prefix("ref: ") {
            let target = sym.trim().to_string();
            if let Some(branch) = target.strip_prefix("refs/heads/") {
                default_branch = branch.to_string();
            }
            ref_map.head = Some(GitHead::Symbolic(target));
        } else if let Ok(spine_id) = ObjectId::from_hex(trimmed) {
            if let Some(&git_oid) = spine_to_git.get(&spine_id) {
                ref_map.head = Some(GitHead::Detached(git_oid));
            }
        }
    }

    Ok((ref_map, default_branch))
}

fn generate_pack_for_wants(
    spine_dir: &Path,
    wants: &[GitOid],
) -> Result<Vec<u8>, GitTransportError> {
    let store = FileSystemObjectStore::new(spine_dir, HashAlgorithm::Blake3, 3)?;
    let tmp_git = spine_dir.join("tmp_git_upload");
    let mut exporter = GitExporter::new(spine_dir, &tmp_git, true)
        .map_err(|e| GitTransportError::Protocol(e.to_string()))?;

    let mut pack_objects = Vec::new();
    let mut visited_git_oids = HashSet::new();
    let mut visited_commits = HashSet::new();
    let mut visited_trees = HashSet::new();
    let mut visited_blobs = HashSet::new();

    let (git_to_spine, _) = load_git_map(spine_dir);

    for want in wants {
        let has_raw = collect_raw_git_objects_recursive(
            spine_dir,
            want,
            &mut pack_objects,
            &mut visited_git_oids,
        );
        if !has_raw {
            if let Some(&spine_id) = git_to_spine.get(want) {
                collect_spine_commit_objects(
                    &store,
                    &spine_id,
                    &mut exporter,
                    &mut pack_objects,
                    &mut visited_commits,
                    &mut visited_trees,
                    &mut visited_blobs,
                )?;
            }
        }
    }

    let (pack_bytes, _idx_bytes, _pack_oid) = build_pack_and_index(pack_objects)?;
    Ok(pack_bytes)
}

fn collect_spine_commit_objects(
    store: &FileSystemObjectStore,
    commit_id: &ObjectId,
    exporter: &mut GitExporter,
    out: &mut Vec<PackObjectToBuild>,
    visited_commits: &mut HashSet<ObjectId>,
    visited_trees: &mut HashSet<ObjectId>,
    visited_blobs: &mut HashSet<ObjectId>,
) -> Result<(), GitTransportError> {
    if !visited_commits.insert(*commit_id) {
        return Ok(());
    }

    let (obj_type, payload) = store.read_object(commit_id)?;
    if obj_type != ObjectType::Commit {
        return Ok(());
    }
    let commit = deserialize_commit(&payload)?;

    // Recursively collect tree
    collect_spine_tree_objects(
        store,
        &commit.root_tree,
        exporter,
        out,
        visited_trees,
        visited_blobs,
    )?;

    // Recursively collect parents
    for parent in &commit.parents {
        collect_spine_commit_objects(
            store,
            parent,
            exporter,
            out,
            visited_commits,
            visited_trees,
            visited_blobs,
        )?;
    }

    // Export Git commit object
    let git_commit_oid = exporter
        .export_commit_recursive_pub(store, commit_id, &mut ExportStats::default())
        .map_err(|e| GitTransportError::Protocol(e.to_string()))?;

    let git_tree_oid = exporter
        .export_tree_recursive_pub(store, &commit.root_tree, &mut ExportStats::default())
        .map_err(|e| GitTransportError::Protocol(e.to_string()))?;

    let mut parent_git_oids = Vec::new();
    for p in &commit.parents {
        if let Ok(p_oid) =
            exporter.export_commit_recursive_pub(store, p, &mut ExportStats::default())
        {
            parent_git_oids.push(p_oid);
        }
    }

    let author_line = format!(
        "{} <{}> {} +0000",
        commit.author_name,
        commit.author_email,
        commit.timestamp_nanos / 1_000_000_000
    );
    let committer_line = format!(
        "{} <{}> {} +0000",
        commit.committer_name,
        commit.committer_email,
        commit.timestamp_nanos / 1_000_000_000
    );

    let git_commit = crate::git_format::GitCommit {
        tree: git_tree_oid,
        parents: parent_git_oids,
        author: author_line,
        committer: committer_line,
        gpgsig: None,
        message: commit.message,
    };
    let commit_payload = crate::git_format::serialize_git_commit(&git_commit);
    out.push(PackObjectToBuild {
        obj_type: GitObjectType::Commit,
        payload: commit_payload,
        oid: git_commit_oid,
    });

    Ok(())
}

fn collect_spine_tree_objects(
    store: &FileSystemObjectStore,
    tree_id: &ObjectId,
    exporter: &mut GitExporter,
    out: &mut Vec<PackObjectToBuild>,
    visited_trees: &mut HashSet<ObjectId>,
    visited_blobs: &mut HashSet<ObjectId>,
) -> Result<(), GitTransportError> {
    if !visited_trees.insert(*tree_id) {
        return Ok(());
    }

    let (obj_type, payload) = store.read_object(tree_id)?;
    if obj_type != ObjectType::Tree {
        return Ok(());
    }
    let entries = deserialize_tree(&payload)?;

    let mut git_entries = Vec::with_capacity(entries.len());
    for entry in entries {
        match entry.mode {
            FileMode::Directory => {
                collect_spine_tree_objects(
                    store,
                    &entry.object_id,
                    exporter,
                    out,
                    visited_trees,
                    visited_blobs,
                )?;
                let subtree_oid = exporter
                    .export_tree_recursive_pub(store, &entry.object_id, &mut ExportStats::default())
                    .map_err(|e| GitTransportError::Protocol(e.to_string()))?;
                git_entries.push(crate::git_format::GitTreeEntry {
                    mode: 0o040000,
                    name: entry.name,
                    oid: subtree_oid,
                });
            }
            FileMode::Regular | FileMode::Executable | FileMode::Symlink => {
                let mode = match entry.mode {
                    FileMode::Executable => 0o100755,
                    FileMode::Symlink => 0o120000,
                    _ => 0o100644,
                };
                if visited_blobs.insert(entry.object_id) {
                    let (_, blob_data) = store.read_object(&entry.object_id)?;
                    let blob_oid = GitOid::compute_object("blob", &blob_data);
                    out.push(PackObjectToBuild {
                        obj_type: GitObjectType::Blob,
                        payload: blob_data,
                        oid: blob_oid,
                    });
                }
                let blob_oid = exporter
                    .export_blob_pub(store, &entry.object_id, &mut ExportStats::default())
                    .map_err(|e| GitTransportError::Protocol(e.to_string()))?;
                git_entries.push(crate::git_format::GitTreeEntry {
                    mode,
                    name: entry.name,
                    oid: blob_oid,
                });
            }
        }
    }

    let tree_payload = crate::git_format::serialize_git_tree(git_entries);
    let tree_oid = GitOid::compute_object("tree", &tree_payload);
    out.push(PackObjectToBuild {
        obj_type: GitObjectType::Tree,
        payload: tree_payload,
        oid: tree_oid,
    });

    Ok(())
}

fn ingest_tree_recursive(
    oid: GitOid,
    raw_objects: &HashMap<GitOid, (GitObjectType, Vec<u8>)>,
    git_to_spine: &mut HashMap<GitOid, ObjectId>,
    spine_to_git: &mut HashMap<ObjectId, GitOid>,
    store: &FileSystemObjectStore,
) -> Result<ObjectId, GitTransportError> {
    if let Some(&spine_id) = git_to_spine.get(&oid) {
        return Ok(spine_id);
    }

    let (_, payload) = raw_objects
        .get(&oid)
        .ok_or(GitTransportError::UnresolvedOid(oid))?;
    let git_entries =
        crate::git_format::parse_git_tree(payload).map_err(|e| GitTransportError::Protocol(e.to_string()))?;

    let mut spine_entries = Vec::with_capacity(git_entries.len());
    for entry in git_entries {
        let (file_mode, child_spine_id) = match entry.mode {
            0o040000 => {
                let id = ingest_tree_recursive(
                    entry.oid,
                    raw_objects,
                    git_to_spine,
                    spine_to_git,
                    store,
                )?;
                (FileMode::Directory, id)
            }
            0o100755 => {
                let id = *git_to_spine
                    .get(&entry.oid)
                    .ok_or(GitTransportError::UnresolvedOid(entry.oid))?;
                (FileMode::Executable, id)
            }
            0o120000 => {
                let id = *git_to_spine
                    .get(&entry.oid)
                    .ok_or(GitTransportError::UnresolvedOid(entry.oid))?;
                (FileMode::Symlink, id)
            }
            _ => {
                let id = *git_to_spine
                    .get(&entry.oid)
                    .ok_or(GitTransportError::UnresolvedOid(entry.oid))?;
                (FileMode::Regular, id)
            }
        };

        spine_entries.push(TreeEntry {
            mode: file_mode,
            name: entry.name,
            object_id: child_spine_id,
        });
    }

    let serialized = serialize_tree(&spine_entries);
    let spine_id = store.write_object(ObjectType::Tree, &serialized)?;
    git_to_spine.insert(oid, spine_id);
    spine_to_git.insert(spine_id, oid);
    Ok(spine_id)
}

fn ingest_commit_recursive(
    oid: GitOid,
    raw_objects: &HashMap<GitOid, (GitObjectType, Vec<u8>)>,
    git_to_spine: &mut HashMap<GitOid, ObjectId>,
    spine_to_git: &mut HashMap<ObjectId, GitOid>,
    store: &FileSystemObjectStore,
) -> Result<Option<(GitOid, ObjectId, Commit)>, GitTransportError> {
    if let Some(&_spine_id) = git_to_spine.get(&oid) {
        return Ok(None);
    }

    let (_, payload) = raw_objects
        .get(&oid)
        .ok_or(GitTransportError::UnresolvedOid(oid))?;
    let git_commit =
        parse_git_commit(payload).map_err(|e| GitTransportError::Protocol(e.to_string()))?;

    // Ingest parent commits
    let mut parent_spine_ids = Vec::new();
    for p in &git_commit.parents {
        if let Some(&p_id) = git_to_spine.get(p) {
            parent_spine_ids.push(p_id);
        } else if raw_objects.contains_key(p) {
            let _ = ingest_commit_recursive(
                *p,
                raw_objects,
                git_to_spine,
                spine_to_git,
                store,
            )?;
            if let Some(&p_id) = git_to_spine.get(p) {
                parent_spine_ids.push(p_id);
            }
        }
    }

    // Ingest tree
    let root_tree_id = ingest_tree_recursive(
        git_commit.tree,
        raw_objects,
        git_to_spine,
        spine_to_git,
        store,
    )?;

    let (author_name, author_email, author_ts) =
        crate::git_format::commit::parse_identity_line(&git_commit.author);
    let (committer_name, committer_email, committer_ts) =
        crate::git_format::commit::parse_identity_line(&git_commit.committer);

    let spine_commit = Commit::new(
        root_tree_id,
        parent_spine_ids,
        author_name,
        author_email,
        committer_name,
        committer_email,
        author_ts * 1_000_000_000,
        git_commit.message,
    );

    let serialized = serialize_commit(&spine_commit);
    let spine_id = store.write_object(ObjectType::Commit, &serialized)?;
    git_to_spine.insert(oid, spine_id);
    spine_to_git.insert(spine_id, oid);

    Ok(Some((oid, spine_id, spine_commit)))
}
