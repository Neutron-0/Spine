use std::io::{self, BufRead, Write};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FastStreamError {
    #[error("I/O error during fast-stream operation: {0}")]
    Io(#[from] io::Error),
    #[error("Malformed fast-import line: {0}")]
    MalformedLine(String),
    #[error("Invalid data length specification in fast-import: {0}")]
    InvalidDataLength(String),
    #[error("Unexpected EOF while reading data payload of size {0}")]
    UnexpectedEof(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FastFileOp {
    Modify {
        mode: String,
        dataref: String, // e.g. ":1" or sha
        path: String,
    },
    Delete {
        path: String,
    },
    Rename {
        source: String,
        target: String,
    },
    Copy {
        source: String,
        target: String,
    },
    DeleteAll,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastCommit {
    pub target_ref: String,
    pub mark: Option<u64>,
    pub author: Option<String>,
    pub committer: String,
    pub message: String,
    pub from: Option<String>,
    pub merges: Vec<String>,
    pub file_ops: Vec<FastFileOp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastBlob {
    pub mark: Option<u64>,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastTag {
    pub tag_name: String,
    pub from: String,
    pub tagger: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastReset {
    pub target_ref: String,
    pub from: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FastCommand {
    Blob(FastBlob),
    Commit(FastCommit),
    Tag(FastTag),
    Reset(FastReset),
    Checkpoint,
    Progress(String),
}

/// Reads a standard Git fast-import stream and parses commands.
pub fn parse_fast_import_stream<R: BufRead>(
    mut reader: R,
) -> Result<Vec<FastCommand>, FastStreamError> {
    let mut commands = Vec::new();
    let mut line_buf = String::new();
    let mut sub_buf = String::new();

    loop {
        line_buf.clear();
        let bytes_read = reader.read_line(&mut line_buf)?;
        if bytes_read == 0 {
            break;
        }

        let line = line_buf.trim_end_matches(['\r', '\n']);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line == "blob" {
            let mut mark = None;
            // Read next lines until "data <len>"
            loop {
                sub_buf.clear();
                reader.read_line(&mut sub_buf)?;
                let sub = sub_buf.trim_end_matches(['\r', '\n']);
                if let Some(m) = sub.strip_prefix("mark :") {
                    mark = m.parse::<u64>().ok();
                } else if let Some(len_str) = sub.strip_prefix("data ") {
                    let len = len_str
                        .parse::<usize>()
                        .map_err(|_| FastStreamError::InvalidDataLength(len_str.to_string()))?;
                    let mut data = vec![0u8; len];
                    reader.read_exact(&mut data)?;
                    // Fast-import requires a trailing LF after data
                    let mut lf = [0u8; 1];
                    let _ = reader.read(&mut lf);
                    commands.push(FastCommand::Blob(FastBlob { mark, data }));
                    break;
                }
            }
        } else if let Some(ref_name) = line.strip_prefix("commit ") {
            let target_ref = ref_name.trim().to_string();
            let mut mark = None;
            let mut author = None;
            let mut committer = String::new();
            let mut message = String::new();
            let mut from = None;
            let mut merges = Vec::new();
            let mut file_ops = Vec::new();

            loop {
                sub_buf.clear();
                let n = reader.read_line(&mut sub_buf)?;
                if n == 0 {
                    break;
                }
                let sub = sub_buf.trim_end_matches(['\r', '\n']);
                if sub.is_empty() {
                    // Blank line ends the commit command in fast-import
                    break;
                }

                if let Some(m) = sub.strip_prefix("mark :") {
                    mark = m.parse::<u64>().ok();
                } else if let Some(a) = sub.strip_prefix("author ") {
                    author = Some(a.to_string());
                } else if let Some(c) = sub.strip_prefix("committer ") {
                    committer = c.to_string();
                } else if let Some(len_str) = sub.strip_prefix("data ") {
                    let len = len_str
                        .parse::<usize>()
                        .map_err(|_| FastStreamError::InvalidDataLength(len_str.to_string()))?;
                    let mut data = vec![0u8; len];
                    reader.read_exact(&mut data)?;
                    message = String::from_utf8_lossy(&data).to_string();
                    let mut lf = [0u8; 1];
                    let _ = reader.read(&mut lf);
                } else if let Some(f) = sub.strip_prefix("from ") {
                    from = Some(f.to_string());
                } else if let Some(m) = sub.strip_prefix("merge ") {
                    merges.push(m.to_string());
                } else if let Some(m_op) = sub.strip_prefix("M ") {
                    let parts: Vec<&str> = m_op.splitn(3, ' ').collect();
                    if parts.len() == 3 {
                        file_ops.push(FastFileOp::Modify {
                            mode: parts[0].to_string(),
                            dataref: parts[1].to_string(),
                            path: parts[2].to_string(),
                        });
                    }
                } else if let Some(d_op) = sub.strip_prefix("D ") {
                    file_ops.push(FastFileOp::Delete {
                        path: d_op.trim().to_string(),
                    });
                } else if let Some(r_op) = sub.strip_prefix("R ") {
                    let parts: Vec<&str> = r_op.splitn(2, ' ').collect();
                    if parts.len() == 2 {
                        file_ops.push(FastFileOp::Rename {
                            source: parts[0].to_string(),
                            target: parts[1].to_string(),
                        });
                    }
                } else if let Some(c_op) = sub.strip_prefix("C ") {
                    let parts: Vec<&str> = c_op.splitn(2, ' ').collect();
                    if parts.len() == 2 {
                        file_ops.push(FastFileOp::Copy {
                            source: parts[0].to_string(),
                            target: parts[1].to_string(),
                        });
                    }
                } else if sub == "deleteall" {
                    file_ops.push(FastFileOp::DeleteAll);
                }
            }

            commands.push(FastCommand::Commit(FastCommit {
                target_ref,
                mark,
                author,
                committer,
                message,
                from,
                merges,
                file_ops,
            }));
        } else if let Some(tag_name) = line.strip_prefix("tag ") {
            let mut from = String::new();
            let mut tagger = None;
            let mut message = String::new();

            loop {
                sub_buf.clear();
                let n = reader.read_line(&mut sub_buf)?;
                if n == 0 {
                    break;
                }
                let sub = sub_buf.trim_end_matches(['\r', '\n']);
                if sub.is_empty() {
                    break;
                }

                if let Some(f) = sub.strip_prefix("from ") {
                    from = f.to_string();
                } else if let Some(t) = sub.strip_prefix("tagger ") {
                    tagger = Some(t.to_string());
                } else if let Some(len_str) = sub.strip_prefix("data ") {
                    let len = len_str
                        .parse::<usize>()
                        .map_err(|_| FastStreamError::InvalidDataLength(len_str.to_string()))?;
                    let mut data = vec![0u8; len];
                    reader.read_exact(&mut data)?;
                    message = String::from_utf8_lossy(&data).to_string();
                    let mut lf = [0u8; 1];
                    let _ = reader.read(&mut lf);
                    break;
                }
            }

            commands.push(FastCommand::Tag(FastTag {
                tag_name: tag_name.to_string(),
                from,
                tagger,
                message,
            }));
        } else if let Some(res_ref) = line.strip_prefix("reset ") {
            let mut from = None;
            loop {
                sub_buf.clear();
                let n = reader.read_line(&mut sub_buf)?;
                if n == 0 {
                    break;
                }
                let sub = sub_buf.trim_end_matches(['\r', '\n']);
                if sub.is_empty() {
                    break;
                }
                if let Some(f) = sub.strip_prefix("from ") {
                    from = Some(f.to_string());
                }
            }
            commands.push(FastCommand::Reset(FastReset {
                target_ref: res_ref.to_string(),
                from,
            }));
        } else if line == "checkpoint" {
            commands.push(FastCommand::Checkpoint);
        } else if let Some(msg) = line.strip_prefix("progress ") {
            commands.push(FastCommand::Progress(msg.to_string()));
        }
    }

    Ok(commands)
}

/// Serializes fast-stream commands to an output writer.
pub fn write_fast_command<W: Write>(
    writer: &mut W,
    cmd: &FastCommand,
) -> Result<(), FastStreamError> {
    match cmd {
        FastCommand::Blob(b) => {
            writeln!(writer, "blob")?;
            if let Some(m) = b.mark {
                writeln!(writer, "mark :{}", m)?;
            }
            writeln!(writer, "data {}", b.data.len())?;
            writer.write_all(&b.data)?;
            writeln!(writer)?;
        }
        FastCommand::Commit(c) => {
            writeln!(writer, "commit {}", c.target_ref)?;
            if let Some(m) = c.mark {
                writeln!(writer, "mark :{}", m)?;
            }
            if let Some(ref a) = c.author {
                writeln!(writer, "author {}", a)?;
            }
            writeln!(writer, "committer {}", c.committer)?;
            let msg_bytes = c.message.as_bytes();
            writeln!(writer, "data {}", msg_bytes.len())?;
            writer.write_all(msg_bytes)?;
            writeln!(writer)?;
            if let Some(ref f) = c.from {
                writeln!(writer, "from {}", f)?;
            }
            for m in &c.merges {
                writeln!(writer, "merge {}", m)?;
            }
            for op in &c.file_ops {
                match op {
                    FastFileOp::Modify {
                        mode,
                        dataref,
                        path,
                    } => {
                        writeln!(writer, "M {} {} {}", mode, dataref, path)?;
                    }
                    FastFileOp::Delete { path } => {
                        writeln!(writer, "D {}", path)?;
                    }
                    FastFileOp::Rename { source, target } => {
                        writeln!(writer, "R {} {}", source, target)?;
                    }
                    FastFileOp::Copy { source, target } => {
                        writeln!(writer, "C {} {}", source, target)?;
                    }
                    FastFileOp::DeleteAll => {
                        writeln!(writer, "deleteall")?;
                    }
                }
            }
            writeln!(writer)?;
        }
        FastCommand::Tag(t) => {
            writeln!(writer, "tag {}", t.tag_name)?;
            writeln!(writer, "from {}", t.from)?;
            if let Some(ref tagger) = t.tagger {
                writeln!(writer, "tagger {}", tagger)?;
            }
            let msg_bytes = t.message.as_bytes();
            writeln!(writer, "data {}", msg_bytes.len())?;
            writer.write_all(msg_bytes)?;
            writeln!(writer)?;
        }
        FastCommand::Reset(r) => {
            writeln!(writer, "reset {}", r.target_ref)?;
            if let Some(ref f) = r.from {
                writeln!(writer, "from {}", f)?;
            }
            writeln!(writer)?;
        }
        FastCommand::Checkpoint => {
            writeln!(writer, "checkpoint")?;
        }
        FastCommand::Progress(msg) => {
            writeln!(writer, "progress {}", msg)?;
        }
    }
    Ok(())
}
