use anyhow::{bail, Context, Result};
use chrono::{TimeZone, Utc};
use spine_config::SpineConfig;
use spine_core::{AnnotatedTag, ObjectId, ObjectType};
use spine_format::{deserialize_annotated_tag, serialize_annotated_tag};
use spine_object_store::{CompositeObjectStore, ObjectStore};
use std::fs;
use std::path::Path;

pub fn cmd_tag_list(primary_spine_dir: &Path) -> Result<()> {
    let tags_dir = primary_spine_dir.join("refs").join("tags");
    if tags_dir.exists() {
        let mut tag_names = Vec::new();
        for entry in fs::read_dir(tags_dir)?.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            tag_names.push(name);
        }
        tag_names.sort();
        for name in tag_names {
            println!("{}", name);
        }
    }
    Ok(())
}

pub fn cmd_tag_create(
    primary_spine_dir: &Path,
    store: &CompositeObjectStore,
    config: &SpineConfig,
    name: &str,
    target_oid: ObjectId,
    annotate: bool,
    message: Option<String>,
) -> Result<()> {
    if name.is_empty() || name.contains(' ') || name.contains('/') || name.contains('~') {
        bail!("fatal: invalid tag name '{}'", name);
    }

    let tag_file = primary_spine_dir.join("refs").join("tags").join(name);
    if tag_file.exists() {
        bail!("fatal: tag '{}' already exists", name);
    }

    if let Some(parent) = tag_file.parent() {
        fs::create_dir_all(parent)?;
    }

    let is_annotated = annotate || message.is_some();

    if is_annotated {
        let (target_type, _) = store.read_object(&target_oid)?;
        let tagger_name = config
            .user
            .name
            .as_deref()
            .unwrap_or("Spine Developer")
            .to_string();
        let tagger_email = config
            .user
            .email
            .as_deref()
            .unwrap_or("dev@spine.org")
            .to_string();
        let tag_msg = message.unwrap_or_else(|| format!("Tag {}", name));
        let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let tag = AnnotatedTag::new(
            target_oid,
            target_type,
            name.to_string(),
            tagger_name,
            tagger_email,
            now,
            tag_msg,
        );

        let serialized = serialize_annotated_tag(&tag);
        let tag_oid = store
            .write_object(ObjectType::Tag, &serialized)
            .context("Failed to write annotated tag object")?;

        fs::write(&tag_file, format!("{}\n", tag_oid.to_hex()))?;
        println!(
            "Created annotated tag '{}' (tag object {}, points to {})",
            name,
            &tag_oid.to_hex()[..7],
            &target_oid.to_hex()[..7]
        );
    } else {
        fs::write(&tag_file, format!("{}\n", target_oid.to_hex()))?;
        println!(
            "Created tag '{}' (Created lightweight tag) pointing to {}",
            name,
            &target_oid.to_hex()[..7]
        );
    }

    Ok(())
}

pub fn cmd_tag_show(
    primary_spine_dir: &Path,
    store: &CompositeObjectStore,
    name: &str,
) -> Result<()> {
    let tag_file = primary_spine_dir.join("refs").join("tags").join(name);
    if !tag_file.exists() {
        bail!("fatal: tag '{}' not found", name);
    }

    let content = fs::read_to_string(&tag_file)?.trim().to_string();
    let oid = ObjectId::from_hex(&content).context("Invalid tag ref")?;

    let (obj_type, payload) = store.read_object(&oid)?;
    match obj_type {
        ObjectType::Tag => {
            let tag = deserialize_annotated_tag(&payload)?;
            println!("tag {}", tag.tag_name);
            println!("Tagger:     {} <{}>", tag.tagger_name, tag.tagger_email);
            if let Some(dt) = Utc
                .timestamp_opt(tag.timestamp_nanos / 1_000_000_000, 0)
                .single()
            {
                println!("Date:       {}", dt.format("%Y-%m-%d %H:%M:%S UTC"));
            }
            println!(
                "Object:     {} ({:?})",
                tag.target_object.to_hex(),
                tag.target_type
            );
            println!("\n    {}\n", tag.message.replace('\n', "\n    "));
        }
        _ => {
            println!("tag {} (lightweight)", name);
            println!("Points to:  {}", oid.to_hex());
        }
    }

    Ok(())
}

pub fn cmd_tag_verify(
    primary_spine_dir: &Path,
    store: &CompositeObjectStore,
    name: &str,
) -> Result<()> {
    let tag_file = primary_spine_dir.join("refs").join("tags").join(name);
    if !tag_file.exists() {
        bail!("fatal: tag '{}' not found", name);
    }

    let content = fs::read_to_string(&tag_file)?.trim().to_string();
    let oid = ObjectId::from_hex(&content).context("Invalid tag ref")?;

    let (obj_type, payload) = store.read_object(&oid)?;
    if obj_type == ObjectType::Tag {
        let tag = deserialize_annotated_tag(&payload)?;
        if !store.has_object(&tag.target_object) {
            bail!(
                "Verification failed: tag target object {} is missing from object store",
                tag.target_object.to_hex()
            );
        }
        println!("Verified annotated tag '{}':", name);
        println!("  Tag Object:    {} [OK]", oid.to_hex());
        println!(
            "  Target Object: {} ({:?}) [OK]",
            tag.target_object.to_hex(),
            tag.target_type
        );
        println!(
            "  Tagger:        {} <{}>",
            tag.tagger_name, tag.tagger_email
        );
    } else {
        if !store.has_object(&oid) {
            bail!(
                "Verification failed: target object {} is missing",
                oid.to_hex()
            );
        }
        println!(
            "Verified lightweight tag '{}' -> {} [OK]",
            name,
            oid.to_hex()
        );
    }

    Ok(())
}

pub fn cmd_tag_delete(primary_spine_dir: &Path, name: &str) -> Result<()> {
    let tag_file = primary_spine_dir.join("refs").join("tags").join(name);
    if tag_file.exists() {
        fs::remove_file(&tag_file)?;
        println!("Deleted tag '{}'", name);
        Ok(())
    } else {
        bail!("error: tag '{}' not found", name);
    }
}
