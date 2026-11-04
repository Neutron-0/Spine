# Git Smart HTTP Protocol Support

## Protocol Specification

Spine Forge implements the Git Smart HTTP transfer protocol as defined in Git's technical documentation (`http-protocol.txt` and `protocol-common.txt`).

### Endpoints and HTTP Headers

| Operation | Method | Route | Request Content-Type | Response Content-Type | Cache Headers |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Upload-Pack Discovery** | `GET` | `/{owner}/{repo}.git/info/refs?service=git-upload-pack` | N/A | `application/x-git-upload-pack-advertisement` | `no-cache, max-age=0, must-revalidate` |
| **Receive-Pack Discovery** | `GET` | `/{owner}/{repo}.git/info/refs?service=git-receive-pack` | N/A | `application/x-git-receive-pack-advertisement` | `no-cache, max-age=0, must-revalidate` |
| **Upload-Pack Execution** | `POST` | `/{owner}/{repo}.git/git-upload-pack` | `application/x-git-upload-pack-request` | `application/x-git-upload-pack-result` | `no-cache, max-age=0, must-revalidate` |
| **Receive-Pack Execution** | `POST` | `/{owner}/{repo}.git/git-receive-pack` | `application/x-git-receive-pack-request` | `application/x-git-receive-pack-result` | `no-cache, max-age=0, must-revalidate` |

*Note: The `.git` suffix is optional on all routes.*

### Packet-Line (`pkt-line`) Codec

All commands, ref advertisements, and capabilities utilize Git 4-byte hexadecimal length-prefixed framing:

- `0000`: Flush packet line (`PktLine::Flush`).
- `0001`: Delimiter packet line (`PktLine::Delimiter`).
- `0002`: Response end packet line (`PktLine::ResponseEnd`).
- `0004`–`fff0`: Payload packet lines (`PktLine::Data(payload)`).

### Capabilities Advertised

#### `git-upload-pack`
- `symref=HEAD:refs/heads/{default_branch}`: Identifies canonical default branch.
- `multi_ack_detailed`: Detailed object negotiation.
- `no-done`: Allows optimization of round-trips.
- `side-band-64k`: Demultiplexes pack data (channel 1), progress info (channel 2), and error messages (channel 3) in 64 KiB chunks.
- `ofs-delta`: Allows offset deltas in pack generation.
- `agent=spine/0.1.0`: Spine Forge user-agent string.

#### `git-receive-pack`
- `report-status`: Client requests status reports for each ref update.
- `delete-refs`: Allows remote ref deletion (e.g. `git push origin --delete branch`).
- `ofs-delta`: Permits offset deltas in incoming packs.
- `agent=spine/0.1.0`: Spine Forge user-agent string.

### Sideband Framing

In `git-upload-pack`, when `side-band-64k` or `side-band` is negotiated:
- Packfile stream is divided into chunks up to 65,515 bytes.
- Packets are formatted as: `[4 bytes hex len][1 byte channel (0x01)][chunk data]`.
- Output is finalized with a flush packet `0000`.
