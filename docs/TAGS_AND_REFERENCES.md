# Spine Tags and References Specification

## Overview
Spine supports both lightweight references and cryptographic annotated tag objects.

## Tag Types

### 1. Lightweight Tags
A lightweight tag is a symbolic reference file stored at `<primary_spine_dir>/refs/tags/<name>` containing a 64-character hexadecimal object ID (commit, tree, or blob).

```bash
spine tag create v1.0.0
```

### 2. Annotated Tags (`ObjectType::Tag = 0x04`)
An annotated tag is an immutable object in the object database containing tagger metadata, creation timestamp, and a release message.

Format specification:
- **Magic**: `b"SPTG"` (4 bytes)
- **Format Version**: `0x0001` (u16 big-endian)
- **Target Object**: 32 bytes (`ObjectId`)
- **Target Type**: 1 byte (`ObjectType`)
- **Tag Name**: Length-prefixed string
- **Tagger Name & Email**: Length-prefixed strings
- **Timestamp**: 8 bytes signed nanoseconds (`i64` big-endian)
- **Message**: Length-prefixed string

```bash
spine tag create v1.0.0 -a -m "Official Spine release v1.0.0"
```

The tag reference in `refs/tags/<name>` points to the hash of the `AnnotatedTag` object.

## Tag Operations

- `spine tag`: Lists all local tags sorted alphabetically.
- `spine tag show <name>`: Displays annotated tag metadata (tagger, date, message) and peel-target commit info.
- `spine tag verify <name>`: Validates that the tag points to an intact object within the object database.
- `spine tag delete <name>`: Removes the tag reference file.

## Garbage Collection Reachability
During reachability analysis, `spine gc` inspects every reference in `refs/tags/`. If a tag ref points to an `ObjectType::Tag` object, GC deserializes the tag and marks its `target_object` as reachable, ensuring tagged commits, trees, and blobs are never collected.
