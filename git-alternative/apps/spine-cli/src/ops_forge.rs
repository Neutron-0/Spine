use crate::ops_credentials::{mask_token, HostCredentials, OsCredentialManager};
use anyhow::{bail, Context, Result};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub fn load_credentials(host: &str) -> Result<Option<HostCredentials>> {
    let mgr = OsCredentialManager::default_manager()?;
    mgr.get_credentials(host)
}

pub fn save_credentials(creds: HostCredentials) -> Result<()> {
    let mgr = OsCredentialManager::default_manager()?;
    mgr.save_credentials(creds)
}

pub fn clear_credentials(host: &str) -> Result<()> {
    let mgr = OsCredentialManager::default_manager()?;
    mgr.delete_credentials(host)
}

/// Lightweight async HTTP 1.1 and HTTPS client supporting certificate validation.
pub async fn http_request(
    method: &str,
    url_str: &str,
    body: Option<&str>,
    token: Option<&str>,
) -> Result<(u16, String)> {
    let trimmed = url_str.trim();
    let is_https = trimmed.starts_with("https://");
    let is_http = trimmed.starts_with("http://");

    if !is_http && !is_https {
        bail!("URL must start with http:// or https:// (got {})", url_str);
    }

    let rest = if is_https {
        trimmed.strip_prefix("https://").unwrap()
    } else {
        trimmed.strip_prefix("http://").unwrap()
    };

    let (host_port, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };

    let (host, port) = if let Some(idx) = host_port.find(':') {
        let h = &host_port[..idx];
        let p: u16 = host_port[idx + 1..].parse().context("Invalid port")?;
        (h, p)
    } else {
        (host_port, if is_https { 443 } else { 80 })
    };

    let connect_addr = format!("{}:{}", host, port);
    let tcp_stream = TcpStream::connect(&connect_addr)
        .await
        .with_context(|| format!("Connecting to {}", connect_addr))?;

    let mut req_str = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: spine-cli/0.1.0\r\nAccept: application/json\r\nConnection: close\r\n",
        method, path, host
    );

    if let Some(tok) = token {
        req_str.push_str(&format!("Authorization: Bearer {}\r\n", tok));
    }

    if let Some(b) = body {
        req_str.push_str("Content-Type: application/json\r\n");
        req_str.push_str(&format!("Content-Length: {}\r\n\r\n", b.len()));
        req_str.push_str(b);
    } else {
        req_str.push_str("\r\n");
    }

    let resp_bytes = if is_https {
        let server_name = rustls_pki_types::ServerName::try_from(host.to_string())
            .map_err(|e| anyhow::anyhow!("Invalid server name {}: {:?}", host, e))?;

        let root_store = rustls::RootCertStore::empty();
        // In local development or testing without webpki-roots, build client config
        let client_cfg = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        let connector = tokio_rustls::TlsConnector::from(Arc::new(client_cfg));
        let mut tls_stream = connector
            .connect(server_name, tcp_stream)
            .await
            .with_context(|| format!("TLS handshake failed with {}", host))?;

        tls_stream.write_all(req_str.as_bytes()).await?;
        tls_stream.flush().await?;

        let mut buf = Vec::new();
        tls_stream.read_to_end(&mut buf).await?;
        buf
    } else {
        let mut stream = tcp_stream;
        stream.write_all(req_str.as_bytes()).await?;
        stream.flush().await?;

        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await?;
        buf
    };

    let resp_str = String::from_utf8_lossy(&resp_bytes);
    let mut parts = resp_str.split("\r\n\r\n");
    let header_part = parts.next().unwrap_or("");
    let body_part = parts.next().unwrap_or("").to_string();

    let status_code: u16 = header_part
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(500);

    Ok((status_code, body_part))
}

pub async fn get_capability_ticket(
    forge_host: &str,
    repo_owner: &str,
    repo_name: &str,
    operation: &str,
) -> Result<String> {
    let creds = load_credentials(forge_host)?.ok_or_else(|| {
        anyhow::anyhow!(
            "Not logged in to {}. Run `spine auth login` first.",
            forge_host
        )
    })?;

    let url = format!(
        "{}/api/v1/repos/{}/{}/capability",
        forge_host.trim_end_matches('/'),
        repo_owner,
        repo_name
    );
    let payload = serde_json::json!({ "operation": operation }).to_string();
    let (status, body) = http_request("POST", &url, Some(&payload), Some(&creds.token)).await?;
    if status != 200 {
        anyhow::bail!(
            "Failed to obtain capability ticket (status {}): {}",
            status,
            body
        );
    }
    let parsed: serde_json::Value = serde_json::from_str(&body)?;
    let ticket = parsed["ticket"]
        .as_str()
        .context("Missing ticket in response")?;
    Ok(ticket.to_string())
}

/// Resolves remote authentication credentials or signed capability ticket for a given Spine URL.
pub async fn resolve_remote_auth(url_str: &str, operation: &str) -> Option<String> {
    if let Ok(ticket) = std::env::var("SPINE_CAPABILITY_TICKET") {
        if !ticket.trim().is_empty() {
            return Some(format!("ticket:{}", ticket.trim()));
        }
    }

    let parsed = spine_net::url::SpineUrl::parse(url_str).ok()?;
    if let Some(user) = &parsed.user {
        if user.starts_with("ticket:") {
            return Some(user.clone());
        }
    }

    let segments: Vec<&str> = parsed.path.trim_matches('/').split('/').collect();
    if segments.len() >= 2 {
        let owner = segments[0];
        let repo = segments[1];

        let candidate_hosts = vec![
            format!("http://{}:8080", parsed.host),
            format!("https://{}", parsed.host),
            parsed.host.clone(),
        ];

        for host in candidate_hosts {
            if let Ok(ticket) = get_capability_ticket(&host, owner, repo, operation).await {
                return Some(format!("ticket:{}", ticket));
            }
        }
    }

    None
}

pub async fn cmd_auth_login(host: &str, username: Option<&str>, token: Option<&str>) -> Result<()> {
    let clean_host = host.trim_end_matches('/');

    if let Some(tok) = token {
        // Authenticate via token
        let check_url = format!("{}/api/v1/auth/session", clean_host);
        let (status, body) = http_request("GET", &check_url, None, Some(tok)).await?;
        if status != 200 {
            bail!("Authentication failed with status {}: {}", status, body);
        }

        let parsed: serde_json::Value = serde_json::from_str(&body)?;
        let user_name = parsed["user"]["username"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let creds = HostCredentials {
            host: clean_host.to_string(),
            username: user_name.clone(),
            token: tok.to_string(),
            expires_at_nanos: now + (30 * 24 * 3600 * 1_000_000_000),
        };
        save_credentials(creds)?;
        println!(
            "Authentication successful for user '{}' on {} (Credential secured in OS store)",
            user_name, clean_host
        );
        return Ok(());
    }

    let user_str = match username {
        Some(u) => u.to_string(),
        None => {
            println!("Enter Forge username or email:");
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            input.trim().to_string()
        }
    };

    println!("Enter password:");
    let mut pwd = String::new();
    std::io::stdin().read_line(&mut pwd)?;
    let clean_pwd = pwd.trim();

    let login_url = format!("{}/api/v1/auth/login", clean_host);
    let payload = serde_json::json!({
        "login": user_str,
        "password": clean_pwd
    });

    let (status, body) = http_request("POST", &login_url, Some(&payload.to_string()), None).await?;
    if status != 200 {
        bail!("Login failed: {}", body);
    }

    let parsed: serde_json::Value = serde_json::from_str(&body)?;
    let user_name = parsed["user"]["username"]
        .as_str()
        .unwrap_or(&user_str)
        .to_string();
    let token_str = parsed["token"].as_str().context("Missing session token")?;
    let expires = parsed["expires_at_nanos"].as_i64().unwrap_or(0);

    let creds = HostCredentials {
        host: clean_host.to_string(),
        username: user_name.clone(),
        token: token_str.to_string(),
        expires_at_nanos: expires,
    };
    save_credentials(creds)?;
    println!(
        "Authentication successful for user '{}' on {} (Credential secured in OS store)",
        user_name, clean_host
    );
    Ok(())
}

pub async fn cmd_auth_status() -> Result<()> {
    let mgr = OsCredentialManager::default_manager()?;
    println!("=== Spine Forge Authentication Status ===");
    println!("Storage Provider: {}", mgr.active_provider_name());

    let default_host = "http://127.0.0.1:3000";
    let creds_opt = mgr.get_credentials(default_host)?;

    match creds_opt {
        Some(creds) => {
            let check_url = format!("{}/api/v1/auth/session", creds.host);
            let status_res = http_request("GET", &check_url, None, Some(&creds.token)).await;
            match status_res {
                Ok((200, body)) => {
                    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let is_auth = parsed["authenticated"].as_bool().unwrap_or(false);
                    println!("  Host:     {}", creds.host);
                    println!("  User:     {}", creds.username);
                    println!("  Token:    {}", mask_token(&creds.token));
                    println!(
                        "  Status:   {}",
                        if is_auth {
                            "ACTIVE"
                        } else {
                            "EXPIRED / INVALID"
                        }
                    );
                }
                _ => {
                    println!("  Host:     {}", creds.host);
                    println!("  User:     {}", creds.username);
                    println!("  Token:    {}", mask_token(&creds.token));
                    println!("  Status:   UNREACHABLE");
                }
            }
        }
        None => {
            println!(
                "No credentials configured for {}. Run `spine auth login`.",
                default_host
            );
        }
    }
    Ok(())
}

pub async fn cmd_auth_logout(host: Option<&str>) -> Result<()> {
    let target_host = host.unwrap_or("http://127.0.0.1:3000");
    if let Ok(Some(creds)) = load_credentials(target_host) {
        let logout_url = format!("{}/api/v1/auth/logout", target_host);
        let _ = http_request("POST", &logout_url, None, Some(&creds.token)).await;
    }
    clear_credentials(target_host)?;
    println!("Logged out from {}", target_host);
    Ok(())
}

pub async fn cmd_repo_create(
    name: &str,
    description: Option<&str>,
    private: bool,
    durability: &str,
    host: Option<&str>,
) -> Result<()> {
    let target_host = host.unwrap_or("http://127.0.0.1:3000");
    let creds =
        load_credentials(target_host)?.context("Not logged in. Run `spine auth login` first.")?;

    let create_url = format!("{}/api/v1/repos", target_host);
    let payload = serde_json::json!({
        "name": name,
        "description": description.unwrap_or(""),
        "is_private": private,
        "durability_policy": durability
    });

    let (status, body) = http_request(
        "POST",
        &create_url,
        Some(&payload.to_string()),
        Some(&creds.token),
    )
    .await?;

    if status != 201 && status != 200 {
        bail!("Failed to create repository: {}", body);
    }

    let parsed: serde_json::Value = serde_json::from_str(&body)?;
    println!("Repository created successfully!");
    println!("  Name:    {}", parsed["name"]);
    println!("  Owner:   {}", parsed["owner"]);
    println!("  Private: {}", parsed["is_private"]);
    println!(
        "  Clone:   spine://127.0.0.1:4434/{}/{}",
        parsed["owner"], parsed["name"]
    );
    Ok(())
}
