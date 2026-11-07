use super::dispatch::GitSshCommand;
use super::host_key::load_or_generate_host_key;
use crate::api::middleware::AppState;
use crate::api::routes_git_http::{ensure_repo_spine_dir, process_git_receive_pack};
use crate::api::routes_lfs::{generate_remote_auth_token, LFS_SECRET_SALT};
use crate::core::types::{DeployKey, ForgeRepository, User};
use anyhow::{Context, Result};
use axum::body::Bytes;
use russh::server::{Auth, Handler, PublicKeyOffer, Server, Session, VerifiedPublicKey};
use russh::ChannelId;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum SshIdentity {
    User(User),
    DeployKey(DeployKey),
}

impl SshIdentity {
    pub fn actor_id(&self) -> String {
        match self {
            SshIdentity::User(u) => u.id.clone(),
            SshIdentity::DeployKey(d) => format!("deploy_key:{}", d.id),
        }
    }
}

pub enum ChannelState {
    UploadPack {
        repo: ForgeRepository,
        in_buf: Vec<u8>,
        finished: bool,
    },
    ReceivePack {
        repo: ForgeRepository,
        actor_id: String,
        in_buf: Vec<u8>,
        finished: bool,
    },
}

pub struct SshServer {
    pub state: AppState,
}

impl Server for SshServer {
    type Handler = SshHandler;

    fn new_client(&mut self, _peer_addr: Option<SocketAddr>) -> Self::Handler {
        SshHandler {
            state: self.state.clone(),
            identity: None,
            channels: HashMap::new(),
        }
    }
}

pub struct SshHandler {
    pub state: AppState,
    pub identity: Option<SshIdentity>,
    pub channels: HashMap<ChannelId, ChannelState>,
}

impl Handler for SshHandler {
    type Error = russh::Error;

    async fn auth_publickey_offered(
        &mut self,
        _user: &str,
        offer: PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
        let fingerprint = offer
            .public_key
            .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
            .to_string();

        if let Ok(Some(_)) = self
            .state
            .db
            .get_user_ssh_key_by_fingerprint(&fingerprint)
            .await
        {
            return Ok(Auth::Accept);
        }

        if let Ok(Some(_)) = self
            .state
            .db
            .get_deploy_key_by_fingerprint(&fingerprint)
            .await
        {
            return Ok(Auth::Accept);
        }

        Ok(Auth::Reject {
            proceed_with_methods: None,
            partial_success: false,
        })
    }

    async fn auth_publickey(
        &mut self,
        _user: &str,
        public_key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        let key = public_key.public_key;
        let fingerprint = key
            .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
            .to_string();

        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

        // 1. Check user SSH keys
        if let Ok(Some(user_key)) = self
            .state
            .db
            .get_user_ssh_key_by_fingerprint(&fingerprint)
            .await
        {
            if let Ok(Some(user)) = self.state.db.get_user_by_id(&user_key.user_id).await {
                let _ = self
                    .state
                    .db
                    .update_user_ssh_key_last_used(&user_key.id, now)
                    .await;
                self.identity = Some(SshIdentity::User(user));
                return Ok(Auth::Accept);
            }
        }

        // 2. Check repository deploy keys
        if let Ok(Some(deploy_key)) = self
            .state
            .db
            .get_deploy_key_by_fingerprint(&fingerprint)
            .await
        {
            let _ = self
                .state
                .db
                .update_deploy_key_last_used(&deploy_key.id, now)
                .await;
            self.identity = Some(SshIdentity::DeployKey(deploy_key));
            return Ok(Auth::Accept);
        }

        Ok(Auth::Reject {
            proceed_with_methods: None,
            partial_success: false,
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: russh::Channel<russh::server::Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let _ = reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let _ = session.channel_failure(channel);
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let _ = session.channel_failure(channel);
        let _ = session.extended_data(
            channel,
            1,
            Bytes::from_static(b"Interactive shell sessions are not permitted on Spine Forge.\n"),
        );
        let _ = session.exit_status_request(channel, 1);
        let _ = session.close(channel);
        Ok(())
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        _name: &str,
        _value: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let _ = session.channel_success(channel);
        Ok(())
    }

    async fn tcpip_forward(
        &mut self,
        _address: &str,
        _port: &mut u32,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn cancel_tcpip_forward(
        &mut self,
        _address: &str,
        _port: u32,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let identity = match &self.identity {
            Some(id) => id.clone(),
            None => {
                let _ = session.channel_failure(channel);
                let _ = session.extended_data(
                    channel,
                    1,
                    Bytes::from_static(b"FATAL: Unauthenticated SSH connection.\n"),
                );
                let _ = session.exit_status_request(channel, 1);
                let _ = session.close(channel);
                return Ok(());
            }
        };

        let cmd_str = match std::str::from_utf8(data) {
            Ok(s) => s,
            Err(_) => {
                let _ = session.channel_failure(channel);
                let _ = session.extended_data(
                    channel,
                    1,
                    Bytes::from_static(b"FATAL: Command is not valid UTF-8.\n"),
                );
                let _ = session.exit_status_request(channel, 1);
                let _ = session.close(channel);
                return Ok(());
            }
        };

        let parsed_cmd = match GitSshCommand::parse(cmd_str) {
            Ok(c) => c,
            Err(e) => {
                let _ = session.channel_success(channel);
                let _ = session.extended_data(
                    channel,
                    1,
                    Bytes::from(format!("FATAL: {}\n", e).into_bytes()),
                );
                let _ = session.exit_status_request(channel, 1);
                let _ = session.close(channel);
                return Ok(());
            }
        };

        match parsed_cmd {
            GitSshCommand::LfsAuthenticate {
                owner,
                repo,
                operation,
            } => {
                let repo_obj = match self
                    .state
                    .db
                    .get_repository_by_owner_name(&owner, &repo)
                    .await
                {
                    Ok(Some(r)) => r,
                    _ => {
                        let _ = session.channel_success(channel);
                        let _ = session.extended_data(
                            channel,
                            1,
                            Bytes::from_static(b"FATAL: Repository not found.\n"),
                        );
                        let _ = session.exit_status_request(channel, 1);
                        let _ = session.close(channel);
                        return Ok(());
                    }
                };

                // Check permissions
                let authorized = match &identity {
                    SshIdentity::User(u) => {
                        if operation == "upload" {
                            self.state
                                .permissions
                                .check_write_access(Some(u), &repo_obj)
                                .await
                                .is_ok()
                        } else {
                            self.state
                                .permissions
                                .check_read_access(Some(u), &repo_obj)
                                .await
                                .is_ok()
                        }
                    }
                    SshIdentity::DeployKey(d) => {
                        if d.repository_id != repo_obj.id {
                            false
                        } else if operation == "upload" {
                            d.write_enabled
                        } else {
                            true
                        }
                    }
                };

                if !authorized {
                    let _ = session.channel_success(channel);
                    let _ = session.extended_data(
                        channel,
                        1,
                        Bytes::from_static(b"FATAL: Permission denied for LFS operation.\n"),
                    );
                    let _ = session.exit_status_request(channel, 1);
                    let _ = session.close(channel);
                    return Ok(());
                }

                let exp_sec = chrono::Utc::now().timestamp() + 3600;
                let token = generate_remote_auth_token(
                    LFS_SECRET_SALT,
                    &identity.actor_id(),
                    &repo_obj.id,
                    &operation,
                    exp_sec,
                );
                let expires_at = chrono::DateTime::from_timestamp(exp_sec, 0)
                    .unwrap_or_default()
                    .to_rfc3339();

                let base_url = self.state.public_origin.trim_end_matches('/');
                let href = format!("{}/{}/{}.git/info/lfs", base_url, owner, repo);

                let json_resp = serde_json::json!({
                    "header": {
                        "Authorization": format!("RemoteAuth {}", token)
                    },
                    "href": href,
                    "expires_at": expires_at
                });

                let json_bytes = serde_json::to_vec(&json_resp).unwrap_or_default();
                let _ = session.channel_success(channel);
                let _ = session.data(channel, Bytes::from(json_bytes));
                let _ = session.data(channel, Bytes::from_static(b"\n"));
                let _ = session.exit_status_request(channel, 0);
                let _ = session.close(channel);
                Ok(())
            }

            GitSshCommand::UploadPack { owner, repo } => {
                let repo_obj = match self
                    .state
                    .db
                    .get_repository_by_owner_name(&owner, &repo)
                    .await
                {
                    Ok(Some(r)) => r,
                    _ => {
                        let _ = session.channel_success(channel);
                        let _ = session.extended_data(
                            channel,
                            1,
                            Bytes::from_static(b"FATAL: Repository not found.\n"),
                        );
                        let _ = session.exit_status_request(channel, 1);
                        let _ = session.close(channel);
                        return Ok(());
                    }
                };

                let authorized = match &identity {
                    SshIdentity::User(u) => self
                        .state
                        .permissions
                        .check_read_access(Some(u), &repo_obj)
                        .await
                        .is_ok(),
                    SshIdentity::DeployKey(d) => d.repository_id == repo_obj.id,
                };

                if !authorized {
                    let _ = session.channel_success(channel);
                    let _ = session.extended_data(
                        channel,
                        1,
                        Bytes::from_static(b"FATAL: Read access denied for repository.\n"),
                    );
                    let _ = session.exit_status_request(channel, 1);
                    let _ = session.close(channel);
                    return Ok(());
                }

                let spine_dir =
                    ensure_repo_spine_dir(self.state.vcs.storage_root(), &repo_obj.native_repo_id);
                let adv = match spine_migration::advertise_refs_upload_pack_raw(&spine_dir) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        let _ = session.channel_failure(channel);
                        let _ = session.extended_data(
                            channel,
                            1,
                            Bytes::from(
                                format!("FATAL: Failed to advertise refs: {}\n", e).into_bytes(),
                            ),
                        );
                        let _ = session.exit_status_request(channel, 1);
                        let _ = session.close(channel);
                        return Ok(());
                    }
                };

                let _ = session.channel_success(channel);
                let _ = session.data(channel, Bytes::from(adv));

                self.channels.insert(
                    channel,
                    ChannelState::UploadPack {
                        repo: repo_obj,
                        in_buf: Vec::new(),
                        finished: false,
                    },
                );

                Ok(())
            }

            GitSshCommand::ReceivePack { owner, repo } => {
                let _write_permit = match self
                    .state
                    .consistency_barrier
                    .acquire_write_permit()
                    .await
                {
                    Ok(p) => p,
                    Err(_) => {
                        let _ = session.channel_success(channel);
                        let _ = session.extended_data(
                            channel,
                            1,
                            Bytes::from_static(b"FATAL: Maintenance in progress. Backups active, repository writes temporarily drained.\n"),
                        );
                        let _ = session.exit_status_request(channel, 1);
                        let _ = session.close(channel);
                        return Ok(());
                    }
                };

                let repo_obj = match self
                    .state
                    .db
                    .get_repository_by_owner_name(&owner, &repo)
                    .await
                {
                    Ok(Some(r)) => r,
                    _ => {
                        let _ = session.channel_success(channel);
                        let _ = session.extended_data(
                            channel,
                            1,
                            Bytes::from_static(b"FATAL: Repository not found.\n"),
                        );
                        let _ = session.exit_status_request(channel, 1);
                        let _ = session.close(channel);
                        return Ok(());
                    }
                };

                let authorized = match &identity {
                    SshIdentity::User(u) => self
                        .state
                        .permissions
                        .check_write_access(Some(u), &repo_obj)
                        .await
                        .is_ok(),
                    SshIdentity::DeployKey(d) => d.repository_id == repo_obj.id && d.write_enabled,
                };

                if !authorized {
                    let _ = session.channel_success(channel);
                    let _ = session.extended_data(
                        channel,
                        1,
                        Bytes::from_static(b"FATAL: Write access denied for repository.\n"),
                    );
                    let _ = session.exit_status_request(channel, 1);
                    let _ = session.close(channel);
                    return Ok(());
                }

                let spine_dir =
                    ensure_repo_spine_dir(self.state.vcs.storage_root(), &repo_obj.native_repo_id);
                let adv = match spine_migration::advertise_refs_receive_pack_raw(&spine_dir) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        let _ = session.channel_failure(channel);
                        let _ = session.extended_data(
                            channel,
                            1,
                            Bytes::from(
                                format!("FATAL: Failed to advertise refs: {}\n", e).into_bytes(),
                            ),
                        );
                        let _ = session.exit_status_request(channel, 1);
                        let _ = session.close(channel);
                        return Ok(());
                    }
                };

                let _ = session.channel_success(channel);
                let _ = session.data(channel, Bytes::from(adv));

                let actor_id = identity.actor_id();
                self.channels.insert(
                    channel,
                    ChannelState::ReceivePack {
                        repo: repo_obj,
                        actor_id,
                        in_buf: Vec::new(),
                        finished: false,
                    },
                );

                Ok(())
            }
        }
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let channel_state = match self.channels.get_mut(&channel) {
            Some(s) => s,
            None => return Ok(()),
        };

        match channel_state {
            ChannelState::UploadPack {
                repo,
                in_buf,
                finished,
            } => {
                if *finished {
                    return Ok(());
                }
                in_buf.extend_from_slice(data);

                // Check if git client sent "done\n" indicating upload-pack negotiation is complete
                if in_buf.windows(4).any(|w| w == b"done") {
                    *finished = true;
                    let spine_dir =
                        ensure_repo_spine_dir(self.state.vcs.storage_root(), &repo.native_repo_id);
                    match spine_migration::handle_upload_pack_request(&spine_dir, in_buf) {
                        Ok(pack_bytes) => {
                            let _ = session.data(channel, Bytes::from(pack_bytes));
                            let _ = session.exit_status_request(channel, 0);
                            let _ = session.close(channel);
                        }
                        Err(e) => {
                            let _ = session.extended_data(
                                channel,
                                1,
                                Bytes::from(format!("Upload-pack failed: {}\n", e).into_bytes()),
                            );
                            let _ = session.exit_status_request(channel, 1);
                            let _ = session.close(channel);
                        }
                    }
                }
            }

            ChannelState::ReceivePack { in_buf, .. } => {
                in_buf.extend_from_slice(data);
            }
        }

        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let channel_state = match self.channels.get_mut(&channel) {
            Some(s) => s,
            None => return Ok(()),
        };

        match channel_state {
            ChannelState::UploadPack {
                repo,
                in_buf,
                finished,
            } => {
                if !*finished {
                    *finished = true;
                    let spine_dir =
                        ensure_repo_spine_dir(self.state.vcs.storage_root(), &repo.native_repo_id);
                    match spine_migration::handle_upload_pack_request(&spine_dir, in_buf) {
                        Ok(pack_bytes) => {
                            let _ = session.data(channel, Bytes::from(pack_bytes));
                            let _ = session.exit_status_request(channel, 0);
                            let _ = session.close(channel);
                        }
                        Err(e) => {
                            let _ = session.extended_data(
                                channel,
                                1,
                                Bytes::from(format!("Upload-pack failed: {}\n", e).into_bytes()),
                            );
                            let _ = session.exit_status_request(channel, 1);
                            let _ = session.close(channel);
                        }
                    }
                }
            }

            ChannelState::ReceivePack {
                repo,
                actor_id,
                in_buf,
                finished,
            } => {
                if !*finished {
                    *finished = true;
                    let res = process_git_receive_pack(&self.state, repo, actor_id, in_buf).await;

                    match res {
                        Ok(report_status) => {
                            let _ = session.data(channel, Bytes::from(report_status));
                            let _ = session.exit_status_request(channel, 0);
                            let _ = session.close(channel);
                        }
                        Err((_code, err_msg)) => {
                            let _ = session.extended_data(
                                channel,
                                1,
                                Bytes::from(err_msg.into_bytes()),
                            );
                            let _ = session.exit_status_request(channel, 1);
                            let _ = session.close(channel);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

pub async fn start_ssh_server(
    state: AppState,
    addr: SocketAddr,
) -> Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let host_key = load_or_generate_host_key(state.vcs.storage_root())?;
    let mut methods = russh::MethodSet::empty();
    methods.push(russh::MethodKind::PublicKey);

    let config = russh::server::Config {
        inactivity_timeout: Some(std::time::Duration::from_secs(300)),
        auth_rejection_time: std::time::Duration::from_millis(50),
        auth_rejection_time_initial: Some(std::time::Duration::from_millis(10)),
        keys: vec![host_key],
        methods,
        ..Default::default()
    };

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind SSH listener to {}", addr))?;
    let local_addr = listener
        .local_addr()
        .context("Failed to get local address of SSH listener")?;

    tracing::info!("Spine Forge SSH daemon listening on {}", local_addr);

    let config = Arc::new(config);
    let mut server = SshServer { state };

    let join_handle = tokio::spawn(async move {
        let running = server.run_on_socket(config, &listener);
        if let Err(e) = running.await {
            tracing::error!("SSH daemon stopped with error: {:?}", e);
        }
    });

    Ok((local_addr, join_handle))
}
