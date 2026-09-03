use super::MsgRtfCompression;

const LZFU_MAGIC: u32 = 0x7546_5a4c;
const MELA_MAGIC: u32 = 0x414c_454d;
const INITIAL_DICTIONARY: &[u8] = b"{\\rtf1\\ansi\\mac\\deff0\\deftab720{\\fonttbl;}{\\f0\\fnil \\froman \\fswiss \\fmodern \\fscript \\fdecor MS Sans SerifSymbolArialTimes New RomanCourier{\\colortbl\\red0\\green0\\blue0\r\n\\par \\pard\\plain\\f0\\fs20\\b\\i\\u\\tab\\tx";

pub(super) fn decompress(
    bytes: &[u8],
    max_output_bytes: usize,
) -> Result<(Vec<u8>, MsgRtfCompression), String> {
    if bytes.len() < 16 {
        return Err("compressed RTF header is truncated".to_string());
    }
    let declared_compressed_size = le_u32(bytes, 0)?;
    let declared_uncompressed_size = le_u32(bytes, 4)?;
    let magic = le_u32(bytes, 8)?;
    let declared_crc32 = le_u32(bytes, 12)?;
    let payload = &bytes[16..];
    if declared_uncompressed_size as usize > max_output_bytes {
        return Err(format!(
            "compressed RTF declares {} output bytes, exceeding max_rtf_output_bytes {max_output_bytes}",
            declared_uncompressed_size
        ));
    }
    let actual_crc32 = crc32(payload);
    let mut output = match magic {
        MELA_MAGIC => payload
            .get(..(declared_uncompressed_size as usize).min(payload.len()))
            .unwrap_or_default()
            .to_vec(),
        LZFU_MAGIC => decompress_lzfu(
            payload,
            declared_uncompressed_size as usize,
            max_output_bytes,
        )?,
        other => return Err(format!("unknown compressed RTF magic 0x{other:08X}")),
    };
    if output.len() > declared_uncompressed_size as usize {
        output.truncate(declared_uncompressed_size as usize);
    }
    let metadata = MsgRtfCompression {
        magic: match magic {
            LZFU_MAGIC => "lzfu",
            MELA_MAGIC => "mela",
            _ => unreachable!(),
        }
        .to_string(),
        declared_compressed_size,
        declared_uncompressed_size,
        declared_crc32,
        actual_crc32,
        crc_matches: declared_crc32 == actual_crc32,
    };
    Ok((output, metadata))
}

fn decompress_lzfu(
    payload: &[u8],
    declared_size: usize,
    max_output_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut dictionary = [0u8; 4096];
    dictionary[..INITIAL_DICTIONARY.len()].copy_from_slice(INITIAL_DICTIONARY);
    let mut write_position = INITIAL_DICTIONARY.len() & 0x0fff;
    let mut input = 0usize;
    let mut output = Vec::with_capacity(declared_size.min(max_output_bytes));
    while input < payload.len() && output.len() < declared_size {
        let flags = payload[input];
        input += 1;
        for bit in 0..8 {
            if output.len() >= declared_size || input >= payload.len() {
                break;
            }
            if flags & (1 << bit) == 0 {
                let value = payload[input];
                input += 1;
                output.push(value);
                dictionary[write_position] = value;
                write_position = (write_position + 1) & 0x0fff;
            } else {
                let Some(pair) = payload.get(input..input + 2) else {
                    return Err("compressed RTF ends inside a dictionary token".to_string());
                };
                input += 2;
                let offset = ((usize::from(pair[0])) << 4) | (usize::from(pair[1]) >> 4);
                let length = usize::from(pair[1] & 0x0f) + 2;
                if offset == write_position {
                    return Ok(output);
                }
                for index in 0..length {
                    if output.len() >= declared_size {
                        break;
                    }
                    if output.len() >= max_output_bytes {
                        return Err(
                            "compressed RTF output exceeded max_rtf_output_bytes".to_string()
                        );
                    }
                    let value = dictionary[(offset + index) & 0x0fff];
                    output.push(value);
                    dictionary[write_position] = value;
                    write_position = (write_position + 1) & 0x0fff;
                }
            }
        }
    }
    if output.len() < declared_size {
        return Err(format!(
            "compressed RTF produced {} of {declared_size} declared bytes",
            output.len()
        ));
    }
    Ok(output)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| "compressed RTF header is truncated".to_string())
}
