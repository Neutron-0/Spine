# SPINE REPOSITORY FORMAT SPECIFICATION v1

## 1. Magic Identifier and Versioning
A Spine repository is designated by a directory named `.spine/` at its root.
- **Magic Identifier:** `SPINEREPO` (8 bytes ASCII: `0x53 0x50 0x49 0x4e 0x45 0x52 0x45 0x50`)
- **Format Version:** `1` (`0x0001` as 16-bit unsigned little-endian integer)
- **Repository Metadata File:** `.spine/spine.toml`
  ```toml
  [repository]
  format_version = 1
  hash_algorithm = "blake3"
  compression = "zstd"
  created_at = 1730332800000000000
  ```

---

## 2. Object Identification & Domain Separation

### 2.1 Hash Algorithms
- `0x01`: BLAKE3 (256-bit / 32 bytes, default)
- `0x02`: SHA-256 (256-bit / 32 bytes, compatibility mode)

### 2.2 Domain Separation Frame
All objects calculate their `ObjectId` over the following structured byte frame:
```
+---------------------------------------------------------------+
| Field            | Type         | Size (bytes) | Description  |
+------------------+--------------+--------------+--------------+
| Magic Prefix     | [u8; 5]      | 5            | b"SPINE"     |
| Null Separator   | u8           | 1            | 0x00         |
| Object Type Byte | u8           | 1            | See below    |
| Null Separator   | u8           | 1            | 0x00         |
| Payload Length   | u64 (LE)     | 8            | Uncompressed |
| Null Separator   | u8           | 1            | 0x00         |
| Payload Bytes    | [u8; length] | N            | Raw content  |
+---------------------------------------------------------------+
```
The resulting 32-byte digest is represented in text hex as a 64-character lowercase hexadecimal string.

---

## 3. Object Types and Binary Encodings

### 3.1 Type Identifiers
- `0x01`: **Blob** (raw file contents)
- `0x02`: **Tree** (directory hierarchy manifest)
- `0x03`: **Commit** (snapshot metadata & DAG parent links)
- `0x04`: **Tag** (annotated reference)

### 3.2 Tree Object Format (`0x02`)
A Tree object contains a sorted array of entries:
```
+-----------------------------------------------------------------------+
| Field            | Type     | Size     | Description                  |
+------------------+----------+----------+------------------------------+
| Entry Count      | u32 (LE) | 4        | Number of tree entries       |
+------------------+----------+----------+------------------------------+
Repeated for each entry (sorted lexicographically by name):
| Mode             | u32 (LE) | 4        | 0o100644 (file), 0o100755, etc|
| Name Length      | u16 (LE) | 2        | Length of UTF-8 path segment |
| Name Bytes       | [u8; N]  | N        | UTF-8 directory entry name   |
| Target ObjectId  | [u8; 32] | 32       | BLAKE3/SHA-256 Object ID     |
+-----------------------------------------------------------------------+
```

### 3.3 Commit Object Format

#### Commit v2 Format (`0x0002` - Default for all new commits)
```
+-----------------------------------------------------------------------+
| Field            | Type     | Size     | Description                  |
+------------------+----------+----------+------------------------------+
| Magic Prefix     | [u8; 4]  | 4        | b"SPCM" (0x53,0x50,0x43,0x4d)|
| Format Version   | u16 (LE) | 2        | 0x0002                       |
| Root Tree Id     | [u8; 32] | 32       | ObjectId of snapshot Tree    |
| Parent Count     | u16 (LE) | 2        | Number of parent commits     |
| Parent ObjectIds | [u8; 32] | 32 * M   | Ordered parent commit ObjectIds|
| Author Name Len  | u16 (LE) | 2        | Length of author name string |
| Author Name      | [u8; N]  | N        | Author name (UTF-8)          |
| Author Email Len | u16 (LE) | 2        | Length of author email string|
| Author Email     | [u8; N]  | N        | Author email (UTF-8)         |
| Committer Name L | u16 (LE) | 2        | Length of committer name     |
| Committer Name   | [u8; N]  | N        | Committer name (UTF-8)       |
| Committer Email L| u16 (LE) | 2        | Length of committer email    |
| Committer Email  | [u8; N]  | N        | Committer email (UTF-8)      |
| Timestamp Nanos  | i64 (LE) | 8        | UTC timestamp in nanoseconds |
| Message Length   | u32 (LE) | 4        | Length of commit message     |
| Message Bytes    | [u8; N]  | N        | Commit message body (UTF-8)  |
+-----------------------------------------------------------------------+
```

#### Commit v1 Format (Legacy Batch 01 Compatibility)
If payload does not start with `b"SPCM"`, it is parsed as v1:
- `root_tree: [u8; 32]`
- `parent_count: u8`
- `parents: [u8; 32] * parent_count`
- `author_name_len: u16 (LE)` + string
- `author_email_len: u16 (LE)` + string
- `timestamp_nanos: i64 (LE)`
- `message_len: u32 (LE)` + string
Committer defaults to Author for all v1 records.

---

## 4. Loose Storage On-Disk Layout
Loose objects are stored under `.spine/objects/<2-hex-prefix>/<62-hex-suffix>`.
- The on-disk file consists of a 16-byte Spine Loose Header followed by the Zstandard compressed frame:
```
+---------------------------------------------------------------+
| Magic: b"SPINEOBJ"     | 8 bytes                              |
| Format Version: 0x0001 | 2 bytes (u16 LE)                     |
| Object Type Byte       | 1 byte                               |
| Reserved               | 5 bytes (0x00)                       |
| Zstandard Frame        | Remaining bytes                      |
+---------------------------------------------------------------+
```

---

## 5. Index File Specification (`.spine/index`)
The index file caches staged changes and working-copy stat information:
```
+---------------------------------------------------------------+
| Magic: b"SPININDX"     | 8 bytes                              |
| Index Version: 0x0001  | 2 bytes (u16 LE)                     |
| Entry Count            | u32 (LE)                             |
| Repeated Entries:                                             |
|   - Path Length        | u16 (LE)                             |
|   - Path String        | UTF-8 bytes                          |
|   - Mode               | u32 (LE)                             |
|   - File Size          | u64 (LE)                             |
|   - MTime Secs         | i64 (LE)                             |
|   - MTime Nanos        | u32 (LE)                             |
|   - Inode Number       | u64 (LE)                             |
|   - Object ID          | [u8; 32]                             |
|   - Staged Status      | u8 (0=Clean, 1=Added/Modified, 2=Del)|
| Trailing Checksum      | 32 bytes (BLAKE3 of preceding index) |
+---------------------------------------------------------------+
```
