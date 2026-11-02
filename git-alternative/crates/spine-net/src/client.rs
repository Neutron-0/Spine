//! Native Spine Wire Protocol client implementation supporting TLS 1.3,
//! Ed25519 cryptographic identity, lease-based force push, and atomic CAS updates.

use crate::auth::{compute_auth_signature, AuthTranscript, DEFAULT_SECRET_KEY};
use crate::identity::SpineIdentity;
use crate::negotiation::compute_missing_objects;
use crate::pack_transfer::{create_pack_file, receive_and_ingest_pack, send_pack_file};
use crate::tls::{SpineStream, TlsError};
use crate::url::SpineUrl;
use spine_core::ObjectId;
use spine_object_store::CompositeObjectStore;
use spine_protocol::{
    frame::{Frame, FrameType, ProtocolError},
    messages::{
        AuthChallenge, AuthResponse, AuthResult, FetchNegotiate, FetchPlan, HelloAckMessage,
        HelloMessage, ProtocolMessage, PushBegin, RefAdvertise, RefUpdateProposal, RefUpdateReq,
        RefUpdateRes, RepoInfo, RepoOpen,
    },
};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;
use tokio::net::TcpStream;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("URL error: {0}")]
    Url(#[from] crate::url::UrlError),
    #[error("TLS error: {0}")]
    Tls(#[from] TlsError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("Transfer error: {0}")]
    Transfer(#[from] crate::pack_transfer::TransferError),
    #[error("Negotiation error: {0}")]
    Negotiation(#[from] crate::negotiation::NegotiationError),
    #[error("Authentication failed: {0}")]
    AuthFailed(String),
    #[error("Plaintext connection forbidden outside loopback for {0}")]
    PlaintextForbiddenOutsideLoopback(String),
    #[error("Server returned error (code {code}): {message}")]
    ServerError { code: u16, message: String },
    #[error("Unexpected frame received: expected {expected:?}, got {actual:?}")]
    UnexpectedFrame {
        expected: FrameType,
        actual: FrameType,
    },
}

/// Connection options configuring TLS, peer identity, and loopback behavior.
#[derive(Clone, Default)]
pub struct ClientConnectOptions {
    pub user_override: Option<String>,
    pub auth_secret: Option<[u8; 32]>,
    pub identity: Option<SpineIdentity>,
    pub tls_config: Option<Arc<rustls::ClientConfig>>,
    pub insecure_plain: bool,
}

#[derive(Debug)]
pub struct SpineClient {
    stream: SpineStream,
    url: SpineUrl,
    repo_info: Option<RepoInfo>,
    advertised_refs: BTreeMap<String, ObjectId>,
}

impl SpineClient {
    /// Connects with default options (allowing loopback plaintext or TLS).
    pub async fn connect(
        url_str: &str,
        user_override: Option<&str>,
        auth_secret: Option<&[u8; 32]>,
    ) -> Result<Self, ClientError> {
        let opts = ClientConnectOptions {
            user_override: user_override.map(|s| s.to_string()),
            auth_secret: auth_secret.copied(),
            identity: None,
            tls_config: None,
            insecure_plain: true,
        };
        Self::connect_with_options(url_str, opts).await
    }

    /// Connects to a remote Spine daemon with full security configuration.
    pub async fn connect_with_options(
        url_str: &str,
        opts: ClientConnectOptions,
    ) -> Result<Self, ClientError> {
        let url = SpineUrl::parse(url_str)?;
        let addr = url.to_address_string();
        let tcp_stream = TcpStream::connect(&addr).await?;

        // Transport negotiation: TLS 1.3 or validated loopback plaintext
        let mut stream = if let Some(tls_cfg) = opts.tls_config {
            SpineStream::upgrade_client(tcp_stream, &url.host, tls_cfg).await?
        } else if url.requires_tls() && !opts.insecure_plain {
            if !url.is_loopback() {
                return Err(ClientError::Tls(TlsError::Config(
                    "TLS 1.3 root certificates required for remote host".to_string(),
                )));
            }
            SpineStream::Plain(tcp_stream)
        } else {
            // Plaintext check
            if !url.is_loopback() {
                return Err(ClientError::PlaintextForbiddenOutsideLoopback(
                    url.host.clone(),
                ));
            }
            SpineStream::Plain(tcp_stream)
        };

        // 1. Handshake HELLO
        let hello = HelloMessage {
            client_version: "0.1.0".to_string(),
            capabilities: vec![
                "pack_zstd".to_string(),
                "ed25519_auth".to_string(),
                "cas_push".to_string(),
                "closure_validation".to_string(),
            ],
            agent: "spine-cli/0.1.0".to_string(),
        };
        hello.to_frame()?.write_to_async(&mut stream).await?;

        let ack_frame = Frame::read_from_async(&mut stream).await?;
        let _hello_ack = HelloAckMessage::from_frame(&ack_frame)?;

        // 2. Authentication challenge
        let challenge_frame = Frame::read_from_async(&mut stream).await?;
        let challenge = AuthChallenge::from_frame(&challenge_frame)?;

        let user = opts
            .user_override
            .as_deref()
            .or(url.user.as_deref())
            .unwrap_or("anonymous");

        let auth_resp = if let Some(id) = &opts.identity {
            // Asymmetric Ed25519 signature
            let transcript = AuthTranscript::new(
                1,
                challenge.server_id.clone(),
                challenge.session_id,
                challenge.nonce,
                challenge.timestamp_ms,
                "".to_string(),
                "default".to_string(),
            );
            let digest = transcript.compute_digest();
            let sig = id.sign(&digest);

            AuthResponse {
                user_id: user.to_string(),
                signature: sig.to_bytes().to_vec(),
                public_key: Some(id.public_key_bytes().to_vec()),
                key_id: Some(id.fingerprint()),
                scope: Some("default".to_string()),
            }
        } else {
            // Symmetric fallback
            let secret = opts.auth_secret.as_ref().unwrap_or(DEFAULT_SECRET_KEY);
            let sig =
                compute_auth_signature(secret, user, &challenge.nonce, challenge.timestamp_ms);

            AuthResponse {
                user_id: user.to_string(),
                signature: sig.to_vec(),
                public_key: None,
                key_id: None,
                scope: None,
            }
        };

        auth_resp.to_frame()?.write_to_async(&mut stream).await?;

        let auth_res_frame = Frame::read_from_async(&mut stream).await?;
        let auth_res = AuthResult::from_frame(&auth_res_frame)?;
        if !auth_res.authenticated {
            return Err(ClientError::AuthFailed(auth_res.message));
        }

        // 3. Open repository
        let repo_open = RepoOpen {
            path: url.path.clone(),
        };
        repo_open.to_frame()?.write_to_async(&mut stream).await?;

        let repo_info_frame = Frame::read_from_async(&mut stream).await?;
        if repo_info_frame.header.frame_type == FrameType::Error {
            let err_msg = spine_protocol::messages::ErrorMessage::from_frame(&repo_info_frame)?;
            return Err(ClientError::ServerError {
                code: err_msg.code,
                message: err_msg.message,
            });
        }
        let repo_info = RepoInfo::from_frame(&repo_info_frame)?;

        // 4. Reference advertisement
        let ref_frame = Frame::read_from_async(&mut stream).await?;
        let ref_adv = RefAdvertise::from_frame(&ref_frame)?;

        Ok(Self {
            stream,
            url,
            repo_info: Some(repo_info),
            advertised_refs: ref_adv.refs,
        })
    }

    pub fn url(&self) -> &SpineUrl {
        &self.url
    }

    pub fn repo_info(&self) -> Option<&RepoInfo> {
        self.repo_info.as_ref()
    }

    pub fn advertised_refs(&self) -> &BTreeMap<String, ObjectId> {
        &self.advertised_refs
    }

    /// Fetches missing objects for `want` commits given local `have` commits,
    /// writing received objects into `dest_store` and registering in `spine_dir`.
    pub async fn fetch(
        &mut self,
        want: &[ObjectId],
        have: &[ObjectId],
        spine_dir: &Path,
        dest_store: &CompositeObjectStore,
    ) -> Result<Option<String>, ClientError> {
        let req = FetchNegotiate {
            want: want.to_vec(),
            have: have.to_vec(),
        };
        req.to_frame()?.write_to_async(&mut self.stream).await?;

        let plan_frame = Frame::read_from_async(&mut self.stream).await?;
        let plan = FetchPlan::from_frame(&plan_frame)?;

        if plan.already_up_to_date || plan.objects_to_send == 0 {
            return Ok(None);
        }

        let pack_id = receive_and_ingest_pack(&mut self.stream, spine_dir, dest_store).await?;
        Ok(pack_id)
    }

    /// Pushes local updates and objects to remote daemon with atomic compare-and-swap and optional lease.
    pub async fn push(
        &mut self,
        updates: &[RefUpdateProposal],
        local_store: &CompositeObjectStore,
        remote_tips: &[ObjectId],
    ) -> Result<RefUpdateRes, ClientError> {
        // Collect commits we want to push
        let want: Vec<ObjectId> = updates.iter().map(|u| u.new_oid).collect();
        let missing = compute_missing_objects(local_store, &want, remote_tips)?;

        let push_begin = PushBegin {
            ref_updates: updates.to_vec(),
            expected_pack_size: None,
        };
        push_begin
            .to_frame()?
            .write_to_async(&mut self.stream)
            .await?;

        if !missing.is_empty() {
            let tmp_pack_path =
                std::env::temp_dir().join(format!("spine-push-{}.spk", uuid::Uuid::new_v4()));
            let (checksum, count) = create_pack_file(local_store, &missing, &tmp_pack_path)?;
            send_pack_file(&tmp_pack_path, checksum, count, &mut self.stream).await?;
            let _ = std::fs::remove_file(&tmp_pack_path);
        } else {
            let empty_end = spine_protocol::messages::PackEnd {
                pack_checksum: [0u8; 32],
                object_count: 0,
            };
            empty_end
                .to_frame()?
                .write_to_async(&mut self.stream)
                .await?;
        }

        // Send RefUpdateReq
        let req = RefUpdateReq {
            updates: updates.to_vec(),
        };
        req.to_frame()?.write_to_async(&mut self.stream).await?;

        // Await RefUpdateRes
        let res_frame = Frame::read_from_async(&mut self.stream).await?;
        let res = RefUpdateRes::from_frame(&res_frame)?;
        Ok(res)
    }
}
