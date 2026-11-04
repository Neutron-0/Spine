use thiserror::Error;

#[derive(Error, Debug)]
pub enum DeltaError {
    #[error("Unexpected EOF while parsing delta stream at byte {0}")]
    UnexpectedEof(usize),
    #[error("Base size mismatch: header declared {declared}, actual base was {actual}")]
    BaseSizeMismatch { declared: usize, actual: usize },
    #[error("Result size mismatch: header declared {declared}, generated {actual}")]
    ResultSizeMismatch { declared: usize, actual: usize },
    #[error("Copy instruction out of bounds: base len {base_len}, requested offset {offset} + size {size}")]
    OutOfBounds {
        base_len: usize,
        offset: usize,
        size: usize,
    },
    #[error("Reserved zero opcode encountered in delta stream")]
    InvalidOpcode,
}

fn read_leb128(data: &[u8], cursor: &mut usize) -> Result<usize, DeltaError> {
    let mut result = 0usize;
    let mut shift = 0;

    loop {
        if *cursor >= data.len() {
            return Err(DeltaError::UnexpectedEof(*cursor));
        }
        let byte = data[*cursor];
        *cursor += 1;

        result |= ((byte & 0x7F) as usize) << shift;
        if (byte & 0x80) == 0 {
            break;
        }
        shift += 7;
        if shift > 64 {
            return Err(DeltaError::UnexpectedEof(*cursor));
        }
    }

    Ok(result)
}

fn write_leb128(mut value: usize, out: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value > 0 {
            byte |= 0x80;
            out.push(byte);
        } else {
            out.push(byte);
            break;
        }
    }
}

/// Applies a Git delta stream to a `base` object to produce the target object.
pub fn apply_delta(base: &[u8], delta: &[u8]) -> Result<Vec<u8>, DeltaError> {
    let mut cursor = 0;
    let base_size = read_leb128(delta, &mut cursor)?;
    let result_size = read_leb128(delta, &mut cursor)?;

    if base.len() != base_size {
        return Err(DeltaError::BaseSizeMismatch {
            declared: base_size,
            actual: base.len(),
        });
    }

    let mut result = Vec::with_capacity(result_size);

    while cursor < delta.len() {
        let op = delta[cursor];
        cursor += 1;

        if (op & 0x80) != 0 {
            // COPY instruction
            let mut offset = 0usize;
            if (op & 0x01) != 0 {
                if cursor >= delta.len() {
                    return Err(DeltaError::UnexpectedEof(cursor));
                }
                offset |= delta[cursor] as usize;
                cursor += 1;
            }
            if (op & 0x02) != 0 {
                if cursor >= delta.len() {
                    return Err(DeltaError::UnexpectedEof(cursor));
                }
                offset |= (delta[cursor] as usize) << 8;
                cursor += 1;
            }
            if (op & 0x04) != 0 {
                if cursor >= delta.len() {
                    return Err(DeltaError::UnexpectedEof(cursor));
                }
                offset |= (delta[cursor] as usize) << 16;
                cursor += 1;
            }
            if (op & 0x08) != 0 {
                if cursor >= delta.len() {
                    return Err(DeltaError::UnexpectedEof(cursor));
                }
                offset |= (delta[cursor] as usize) << 24;
                cursor += 1;
            }

            let mut size = 0usize;
            if (op & 0x10) != 0 {
                if cursor >= delta.len() {
                    return Err(DeltaError::UnexpectedEof(cursor));
                }
                size |= delta[cursor] as usize;
                cursor += 1;
            }
            if (op & 0x20) != 0 {
                if cursor >= delta.len() {
                    return Err(DeltaError::UnexpectedEof(cursor));
                }
                size |= (delta[cursor] as usize) << 8;
                cursor += 1;
            }
            if (op & 0x40) != 0 {
                if cursor >= delta.len() {
                    return Err(DeltaError::UnexpectedEof(cursor));
                }
                size |= (delta[cursor] as usize) << 16;
                cursor += 1;
            }

            if size == 0 {
                size = 0x10000; // 65536
            }

            if offset + size > base.len() {
                return Err(DeltaError::OutOfBounds {
                    base_len: base.len(),
                    offset,
                    size,
                });
            }

            result.extend_from_slice(&base[offset..offset + size]);
        } else if op > 0 {
            // INSERT instruction
            let size = op as usize;
            if cursor + size > delta.len() {
                return Err(DeltaError::UnexpectedEof(cursor));
            }
            result.extend_from_slice(&delta[cursor..cursor + size]);
            cursor += size;
        } else {
            return Err(DeltaError::InvalidOpcode);
        }
    }

    if result.len() != result_size {
        return Err(DeltaError::ResultSizeMismatch {
            declared: result_size,
            actual: result.len(),
        });
    }

    Ok(result)
}

/// Creates a delta stream transforming `base` into `target`.
/// Implements greedy prefix/suffix matching and direct insertion.
pub fn create_delta(base: &[u8], target: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    write_leb128(base.len(), &mut out);
    write_leb128(target.len(), &mut out);

    // If identical, copy whole base
    if base == target && !base.is_empty() {
        let mut offset = 0;
        let mut remaining = base.len();
        while remaining > 0 {
            let chunk = remaining.min(0x10000);
            emit_copy_cmd(offset, chunk, &mut out);
            offset += chunk;
            remaining -= chunk;
        }
        return out;
    }

    // Find common prefix
    let prefix_len = base
        .iter()
        .zip(target.iter())
        .take_while(|(a, b)| a == b)
        .count();

    // Find common suffix outside prefix
    let suffix_len = base[prefix_len..]
        .iter()
        .rev()
        .zip(target[prefix_len..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();

    // Copy prefix if non-empty
    if prefix_len > 0 {
        let mut off = 0;
        let mut rem = prefix_len;
        while rem > 0 {
            let chunk = rem.min(0x10000);
            emit_copy_cmd(off, chunk, &mut out);
            off += chunk;
            rem -= chunk;
        }
    }

    // Insert changed middle section
    let middle_target = &target[prefix_len..target.len() - suffix_len];
    for chunk in middle_target.chunks(127) {
        out.push(chunk.len() as u8);
        out.extend_from_slice(chunk);
    }

    // Copy suffix if non-empty
    if suffix_len > 0 {
        let mut off = base.len() - suffix_len;
        let mut rem = suffix_len;
        while rem > 0 {
            let chunk = rem.min(0x10000);
            emit_copy_cmd(off, chunk, &mut out);
            off += chunk;
            rem -= chunk;
        }
    }

    out
}

fn emit_copy_cmd(offset: usize, size: usize, out: &mut Vec<u8>) {
    let mut op = 0x80u8;
    let mut params = Vec::with_capacity(7);

    // Offset bytes (bits 0..3)
    if (offset & 0xFF) != 0 {
        op |= 0x01;
        params.push((offset & 0xFF) as u8);
    }
    if ((offset >> 8) & 0xFF) != 0 {
        op |= 0x02;
        params.push(((offset >> 8) & 0xFF) as u8);
    }
    if ((offset >> 16) & 0xFF) != 0 {
        op |= 0x04;
        params.push(((offset >> 16) & 0xFF) as u8);
    }
    if ((offset >> 24) & 0xFF) != 0 {
        op |= 0x08;
        params.push(((offset >> 24) & 0xFF) as u8);
    }

    // Size bytes (bits 4..6)
    if size != 0x10000 {
        if (size & 0xFF) != 0 {
            op |= 0x10;
            params.push((size & 0xFF) as u8);
        }
        if ((size >> 8) & 0xFF) != 0 {
            op |= 0x20;
            params.push(((size >> 8) & 0xFF) as u8);
        }
        if ((size >> 16) & 0xFF) != 0 {
            op |= 0x40;
            params.push(((size >> 16) & 0xFF) as u8);
        }
    }

    out.push(op);
    out.extend_from_slice(&params);
}
