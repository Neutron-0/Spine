use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostCredentials {
    pub host: String,
    pub username: String,
    pub token: String,
    pub expires_at_nanos: i64,
}

#[derive(Default, Serialize, Deserialize)]
struct CredentialsFile {
    #[serde(default)]
    hosts: BTreeMap<String, HostCredentials>,
}

fn get_credentials_path() -> Result<PathBuf> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .context("Could not determine user home directory")?;
    let spine_home = PathBuf::from(home).join(".spine");
    fs::create_dir_all(&spine_home)?;
    Ok(spine_home.join("credentials.toml"))
}

fn load_credentials(host: &str) -> Result<Option<HostCredentials>> {
    let path = get_credentials_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)?;
    let creds: CredentialsFile = toml::from_str(&content).unwrap_or_default();
    Ok(creds.hosts.get(host).cloned())
}

fn save_credentials(creds: HostCredentials) -> Result<()> {
    let path = get_credentials_path()?;
    let mut creds_file: CredentialsFile = if path.exists() {
        let content = fs::read_to_string(&path)?;
        toml::from_str(&content).unwrap_or_default()
    } else {
        CredentialsFile::default()
    };

    creds_file.hosts.insert(creds.host.clone(), creds);
    let toml_str = toml::to_string_pretty(&creds_file)?;
    fs::write(&path, toml_str)?;
    Ok(())
}

fn clear_credentials(host: &str) -> Result<()> {
    let path = get_credentials_path()?;
    if path.exists() {
        let content = fs::read_to_string(&path)?;
        let mut creds_file: CredentialsFile = toml::from_str(&content).unwrap_or_default();
        creds_file.hosts.remove(host);
        let toml_str = toml::to_string_pretty(&creds_file)?;
        fs::write(&path, toml_str)?;
    }
    Ok(())
}

/// Lightweight, async HTTP 1.1 client without external HTTP dependencies.
pub async fn http_request(
    method: &str,
    url_str: &str,
    body: Option<&str>,
    token: Option<&str>,
) -> Result<(u16, String)> {
    let trimmed = url_str.trim();
    let rest = trimmed
        .strip_prefix("http://")
        .context("Only http:// URLs supported for local Forge")?;

    let (host_port, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };

    let connect_addr = if host_port.contains(':') {
        host_port.to_string()
    } else {
        format!("{}:80", host_port)
    };

    let mut stream = TcpStream::connect(&connect_addr)
        .await
        .with_context(|| format!("Connecting to {}", connect_addr))?;

    let mut req_str = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: spine-cli/0.1.0\r\nAccept: application/json\r\nConnection: close\r\n",
        method, path, host_port
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

    stream.write_all(req_str.as_bytes()).await?;
    stream.flush().await?;

    let mut resp_bytes = Vec::new();
    stream.read_to_end(&mut resp_bytes).await?;

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

pub async fn cmd_auth_login(
    host: &str,
    username: Option<&str>,
    token: Option<&str>,
) -> Result<()> {
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
            "Authentication successful for user '{}' on {}",
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

    let (status, body) =
        http_request("POST", &login_url, Some(&payload.to_string()), None).await?;
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
        "Authentication successful for user '{}' on {}",
        user_name, clean_host
    );
    Ok(())
}

pub async fn cmd_auth_status() -> Result<()> {
    let path = get_credentials_path()?;
    if !path.exists() {
        println!("No credentials configured. Run `spine auth login`.");
        return Ok(());
    }

    let content = fs::read_to_string(&path)?;
    let creds_file: CredentialsFile = toml::from_str(&content).unwrap_or_default();
    if creds_file.hosts.is_empty() {
        println!("No credentials configured. Run `spine auth login`.");
        return Ok(());
    }

    println!("=== Spine Forge Authentication Status ===");
    for (host, creds) in creds_file.hosts {
        let check_url = format!("{}/api/v1/auth/session", host);
        let status_res = http_request("GET", &check_url, None, Some(&creds.token)).await;
        match status_res {
            Ok((200, body)) => {
                let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                let is_auth = parsed["authenticated"].as_bool().unwrap_or(false);
                if is_auth {
                    println!("  Host:     {}", host);
                    println!("  User:     {}", creds.username);
                    println!("  Status:   ACTIVE");
                } else {
                    println!("  Host:     {}", host);
                    println!("  User:     {}", creds.username);
                    println!("  Status:   EXPIRED / INVALID");
                }
            }
            _ => {
                println!("  Host:     {}", host);
                println!("  User:     {}", creds.username);
                println!("  Status:   UNREACHABLE");
            }
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
    let creds = load_credentials(target_host)?
        .context("Not logged in. Run `spine auth login` first.")?;

    let create_url = format!("{}/api/v1/repos", target_host);
    let payload = serde_json::json!({
        "name": name,
        "description": description,
        "is_private": private,
        "durability": durability
    });

    let (status, body) = http_request(
        "POST",
        &create_url,
        Some(&payload.to_string()),
        Some(&creds.token),
    )
    .await?;

    if status != 201 {
        bail!("Repository creation failed (status {}): {}", status, body);
    }

    let parsed: serde_json::Value = serde_json::from_str(&body)?;
    let owner = parsed["owner"].as_str().unwrap_or(&creds.username);
    let repo_name = parsed["name"].as_str().unwrap_or(name);
    let status_str = parsed["provisioning_status"].as_str().unwrap_or("PENDING");

    println!("=== Repository Created Successfully ===");
    println!("  Repository:   {}/{}", owner, repo_name);
    println!("  Visibility:   {}", if private { "Private" } else { "Public" });
    println!("  Status:       {}", status_str);
    println!("  Clone URL:    spine://127.0.0.1:4434/{}/{}", owner, repo_name);
    Ok(())
}
