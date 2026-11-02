use anyhow::{Context, Result};
use spine_cluster::node::NodeStatus;
use spine_cluster::placement::PlacementScheduler;
use spine_cluster::protocol::{ClusterRpcRequest, ClusterRpcResponse};
use spine_cluster::store::{CoordinatorStore, FileBackedCoordinatorStore};
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub fn default_coordinator_addr() -> String {
    std::env::var("SPINE_COORDINATOR_ADDR").unwrap_or_else(|_| "127.0.0.1:9000".to_string())
}

pub async fn send_coordinator_rpc(
    addr: &str,
    req: &ClusterRpcRequest,
) -> Result<ClusterRpcResponse> {
    let mut stream = TcpStream::connect(addr)
        .await
        .with_context(|| format!("Connecting to coordinator at {}", addr))?;

    let req_bytes = req.to_bytes()?;
    let req_len = (req_bytes.len() as u32).to_be_bytes();
    stream.write_all(&req_len).await?;
    stream.write_all(&req_bytes).await?;
    stream.flush().await?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let resp_len = u32::from_be_bytes(len_buf) as usize;

    let mut resp_body = vec![0u8; resp_len];
    stream.read_exact(&mut resp_body).await?;

    let resp = ClusterRpcResponse::from_bytes(&resp_body)?;
    Ok(resp)
}

fn open_local_or_default_store() -> Result<FileBackedCoordinatorStore> {
    let path = Path::new(".spine-coordinator-state.json");
    FileBackedCoordinatorStore::new_file_backed(path)
}

pub fn cmd_cluster_status(store_opt: Option<&dyn CoordinatorStore>) -> Result<()> {
    let local_store;
    let store: &dyn CoordinatorStore = match store_opt {
        Some(s) => s,
        None => {
            local_store = open_local_or_default_store()?;
            &local_store
        }
    };

    let nodes = store.list_nodes()?;
    let repos = store.list_repositories()?;

    let mut healthy = 0;
    let mut suspect = 0;
    let mut offline = 0;
    let mut quarantined = 0;
    let mut draining = 0;
    let mut total_cap = 0u64;
    let mut used_cap = 0u64;
    let mut avail_cap = 0u64;

    for n in &nodes {
        match n.status {
            NodeStatus::Healthy => healthy += 1,
            NodeStatus::Suspect => suspect += 1,
            NodeStatus::Offline => offline += 1,
            NodeStatus::Quarantined => quarantined += 1,
            NodeStatus::Draining => draining += 1,
            _ => {}
        }
        total_cap += n.capacity.total_bytes;
        used_cap += n.capacity.used_bytes;
        avail_cap += n.capacity.available_bytes;
    }

    println!("===========================================================");
    println!("             SPINE CLUSTER STATUS                          ");
    println!("===========================================================");
    println!("Total Storage Nodes:    {}", nodes.len());
    println!(
        "Node Health Breakdown:  {} healthy | {} suspect | {} offline | {} quarantined | {} draining",
        healthy, suspect, offline, quarantined, draining
    );
    println!("Total Repositories:     {}", repos.len());
    println!(
        "Storage Capacity:       {} total | {} used | {} available",
        format_bytes(total_cap),
        format_bytes(used_cap),
        format_bytes(avail_cap)
    );
    println!("===========================================================");
    Ok(())
}

pub fn cmd_node_list(store_opt: Option<&dyn CoordinatorStore>) -> Result<()> {
    let local_store;
    let store: &dyn CoordinatorStore = match store_opt {
        Some(s) => s,
        None => {
            local_store = open_local_or_default_store()?;
            &local_store
        }
    };

    let nodes = store.list_nodes()?;
    println!(
        "{:<15} {:<12} {:<22} {:<16} {:<10}",
        "NODE ID", "STATUS", "LISTEN ADDR", "CAPACITY (USED/TOT)", "ERRORS"
    );
    println!("{}", "-".repeat(78));

    for n in nodes {
        let cap_str = format!(
            "{}/{}",
            format_bytes(n.capacity.used_bytes),
            format_bytes(n.capacity.total_bytes)
        );
        println!(
            "{:<15} {:<12} {:<22} {:<16} {:<10}",
            n.node_id,
            n.status.to_string(),
            n.listen_addr,
            cap_str,
            n.error_count
        );
    }
    Ok(())
}

pub fn cmd_node_inspect(node_id: &str, store_opt: Option<&dyn CoordinatorStore>) -> Result<()> {
    let local_store;
    let store: &dyn CoordinatorStore = match store_opt {
        Some(s) => s,
        None => {
            local_store = open_local_or_default_store()?;
            &local_store
        }
    };

    let node = store
        .get_node(node_id)?
        .context(format!("Node '{}' not found", node_id))?;

    println!("Node ID:          {}", node.node_id);
    println!("Status:           {}", node.status);
    println!("Public Key:       {}", node.public_key_hex);
    println!("Fingerprint:      {}", node.fingerprint);
    println!("Listen Address:   {}", node.listen_addr);
    println!("Storage Root:     {}", node.storage_root);
    println!(
        "Failure Domain:   region={}, zone={}, host={}",
        node.failure_domain.region, node.failure_domain.zone, node.failure_domain.host
    );
    println!(
        "Total Bytes:      {}",
        format_bytes(node.capacity.total_bytes)
    );
    println!(
        "Used Bytes:       {}",
        format_bytes(node.capacity.used_bytes)
    );
    println!(
        "Available Bytes:  {}",
        format_bytes(node.capacity.available_bytes)
    );
    println!("Active Tasks:     {}", node.active_tasks);
    println!("Error Counter:    {}", node.error_count);
    Ok(())
}

pub fn cmd_node_drain(node_id: &str, store_opt: Option<&dyn CoordinatorStore>) -> Result<()> {
    let local_store;
    let store: &dyn CoordinatorStore = match store_opt {
        Some(s) => s,
        None => {
            local_store = open_local_or_default_store()?;
            &local_store
        }
    };
    store.update_node_status(node_id, NodeStatus::Draining)?;
    println!("Node '{}' marked as DRAINING.", node_id);
    Ok(())
}

pub fn cmd_node_quarantine(node_id: &str, store_opt: Option<&dyn CoordinatorStore>) -> Result<()> {
    let local_store;
    let store: &dyn CoordinatorStore = match store_opt {
        Some(s) => s,
        None => {
            local_store = open_local_or_default_store()?;
            &local_store
        }
    };
    store.update_node_status(node_id, NodeStatus::Quarantined)?;
    println!("Node '{}' marked as QUARANTINED.", node_id);
    Ok(())
}

pub fn cmd_node_retire(node_id: &str, store_opt: Option<&dyn CoordinatorStore>) -> Result<()> {
    let local_store;
    let store: &dyn CoordinatorStore = match store_opt {
        Some(s) => s,
        None => {
            local_store = open_local_or_default_store()?;
            &local_store
        }
    };
    store.update_node_status(node_id, NodeStatus::Retired)?;
    println!("Node '{}' marked as RETIRED.", node_id);
    Ok(())
}

pub fn cmd_storage_placements(store_opt: Option<&dyn CoordinatorStore>) -> Result<()> {
    let local_store;
    let store: &dyn CoordinatorStore = match store_opt {
        Some(s) => s,
        None => {
            local_store = open_local_or_default_store()?;
            &local_store
        }
    };

    let repos = store.list_repositories()?;
    println!(
        "{:<20} {:<12} {:<24} {:<12} {:<6}",
        "REPOSITORY", "PRIMARY", "REPLICAS", "DURABILITY", "GEN"
    );
    println!("{}", "-".repeat(78));

    for r in repos {
        let placement = store.get_placement(&r.repo_id)?.unwrap_or_else(|| {
            spine_cluster::placement::PlacementPlan {
                repo_id: r.repo_id.clone(),
                primary_node_id: "unassigned".into(),
                replica_node_ids: vec![],
                required_copies: r.durability_policy.required_copies(),
                placement_generation: 0,
            }
        });

        println!(
            "{:<20} {:<12} {:<24} {:<12} {:<6}",
            r.repo_id,
            placement.primary_node_id,
            placement.replica_node_ids.join(", "),
            r.durability_policy.to_string(),
            r.committed_generation
        );
    }
    Ok(())
}

pub fn cmd_storage_rebalance(
    dry_run: bool,
    store_opt: Option<&dyn CoordinatorStore>,
) -> Result<()> {
    let local_store;
    let store: &dyn CoordinatorStore = match store_opt {
        Some(s) => s,
        None => {
            local_store = open_local_or_default_store()?;
            &local_store
        }
    };

    let repos = store.list_repositories()?;
    let nodes = store.list_nodes()?;

    println!(
        "Evaluating rebalance across {} repositories and {} storage nodes (dry_run: {})...",
        repos.len(),
        nodes.len(),
        dry_run
    );

    let mut planned = 0;
    for r in &repos {
        let plan = PlacementScheduler::schedule_placement(
            &r.repo_id,
            r.durability_policy,
            &nodes,
            1_000_000,
            r.committed_generation,
        )?;

        let current = store.get_placement(&r.repo_id)?;
        let needs_change = match current {
            Some(ref c) => {
                c.primary_node_id != plan.primary_node_id
                    || c.replica_node_ids != plan.replica_node_ids
            }
            None => true,
        };

        if needs_change {
            planned += 1;
            println!(
                "Repository '{}': Primary -> {}, Replicas -> [{}]",
                r.repo_id,
                plan.primary_node_id,
                plan.replica_node_ids.join(", ")
            );
            if !dry_run {
                store.set_placement(plan)?;
            }
        }
    }

    if planned == 0 {
        println!("Cluster is balanced. No placement modifications required.");
    } else if dry_run {
        println!("Dry run complete: {} placement updates planned.", planned);
    } else {
        println!("Rebalance executed: {} placement plans updated.", planned);
    }

    Ok(())
}

pub fn cmd_replication_status(store_opt: Option<&dyn CoordinatorStore>) -> Result<()> {
    let local_store;
    let store: &dyn CoordinatorStore = match store_opt {
        Some(s) => s,
        None => {
            local_store = open_local_or_default_store()?;
            &local_store
        }
    };

    let txs = store.list_active_transactions()?;
    println!("Active Replication Transactions: {}", txs.len());
    for t in txs {
        println!(
            "  [{}] Repo: {} | Primary: {} | Phase: {} | Prepared: {}/{}",
            t.transaction_id,
            t.repo_id,
            t.primary_node_id,
            t.phase,
            t.prepared_nodes.len(),
            t.policy.required_copies()
        );
    }

    let repairs = store.list_repair_jobs()?;
    println!("Repair Jobs: {}", repairs.len());
    for r in repairs {
        println!(
            "  [{}] Repo: {} | Target: {} | Source: {} | Status: {:?}",
            r.job_id, r.repo_id, r.target_node_id, r.source_node_id, r.status
        );
    }

    Ok(())
}

pub fn cmd_replication_repair(
    repo_id: &str,
    store_opt: Option<&dyn CoordinatorStore>,
) -> Result<()> {
    let local_store;
    let store: &dyn CoordinatorStore = match store_opt {
        Some(s) => s,
        None => {
            local_store = open_local_or_default_store()?;
            &local_store
        }
    };

    let placement = store
        .get_placement(repo_id)?
        .context(format!("Repository '{}' placement not found", repo_id))?;

    let replicas = store.get_replicas(repo_id)?;
    println!(
        "Scanning anti-entropy for repository '{}' (Primary: {})...",
        repo_id, placement.primary_node_id
    );

    let mut repair_count = 0;
    for rep in replicas {
        if !rep.verified {
            let job_id = format!("repair-{}", &uuid::Uuid::new_v4().to_string()[..8]);
            let job = spine_cluster::repair::RepairJob::new(
                job_id.clone(),
                repo_id.to_string(),
                rep.node_id.clone(),
                placement.primary_node_id.clone(),
                vec!["pack-diff-detected".into()],
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            );
            store.create_repair_job(job)?;
            repair_count += 1;
            println!(
                "  Scheduled repair job '{}' for unverified replica '{}'",
                job_id, rep.node_id
            );
        }
    }

    if repair_count == 0 {
        println!("All replicas are verified and in sync.");
    } else {
        println!("Scheduled {} repair jobs.", repair_count);
    }

    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}
