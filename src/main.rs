//! Converts PolkaVM blob format to the JAM PVM blob format.
//!
//! The key difference is that PolkaVM format stores RO/RW data without zero-padding,
//! while JAM format requires padding to the declared sizes.
//!
//! This does use Jan's code on how to decode that Blob format as a reference.

use std::env;
use std::fs;
use std::process;

const MAGIC: &[u8] = b"PVM\0";
const VERSION: u8 = 0;

const SECTION_MEM_CFG: u8 = 1;
const SECTION_RO_DATA: u8 = 2;
const SECTION_RW_DATA: u8 = 3;
const SECTION_CODE_AND_JUMP_TABLE: u8 = 6;

fn main() {
    let args: Vec<String> = env::args().collect();

    let (input_path, output_path) = match args.len() {
        2 => (&args[1], args[1].clone()),
        4 if args[2] == "-o" => (&args[1], args[3].clone()),
        _ => {
            eprintln!("Usage: {} <input.pvm> [-o <output.pvm>]", args[0]);
            eprintln!("Converts PolkaVM blob format to JAM format.");
            eprintln!("If -o is not specified, converts in-place.");
            process::exit(1);
        }
    };

    let data = fs::read(input_path).unwrap_or_else(|e| {
        eprintln!("Failed to read file: {}", e);
        process::exit(1);
    });

    let result = convert(&data).unwrap_or_else(|e| {
        eprintln!("Conversion failed: {}", e);
        process::exit(1);
    });

    fs::write(&output_path, &result).unwrap_or_else(|e| {
        eprintln!("Failed to write file: {}", e);
        process::exit(1);
    });
}

fn convert(data: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut cursor = data;

    // Check magic
    if cursor.len() < 4 || &cursor[..4] != MAGIC {
        return Err("Invalid magic bytes");
    }
    cursor = &cursor[4..];

    // Check version
    if cursor.is_empty() || cursor[0] != VERSION {
        return Err("Invalid version");
    }
    cursor = &cursor[1..];

    // Read data length (u64 LE)
    if cursor.len() < 8 {
        return Err("Missing data length");
    }
    let data_len = u64::from_le_bytes(cursor[..8].try_into().unwrap()) as usize;
    cursor = &cursor[8..];

    if data_len != data.len() {
        return Err("Data length mismatch");
    }

    // Parse memory config section
    let (ro_data_size, rw_data_size, stack_size, rest) = decode_memory_section(cursor)?;
    cursor = rest;

    // Parse RO data section
    let (ro_data, rest) = decode_generic_section(SECTION_RO_DATA, cursor)?;
    cursor = rest;

    // Parse RW data section
    let (rw_data, rest) = decode_generic_section(SECTION_RW_DATA, cursor)?;
    cursor = rest;

    // Skip imports section (4) if present
    if !cursor.is_empty() && cursor[0] == 4 {
        let (_, rest) = decode_imports_section(cursor)?;
        cursor = rest;
    }

    // Skip exports section (5) if present
    if !cursor.is_empty() && cursor[0] == 5 {
        let (_, rest) = decode_generic_section(5, cursor)?;
        cursor = rest;
    }

    // Parse code and jump table section
    let (program, _rest) = decode_generic_section(SECTION_CODE_AND_JUMP_TABLE, cursor)?;

    // Validate sizes
    if ro_data.len() > ro_data_size {
        return Err("RO data larger than declared size");
    }
    if rw_data.len() > rw_data_size {
        return Err("RW data larger than declared size");
    }

    // Build JAM format output
    let mut output = Vec::new();

    // Header: ro_data_size (u24), rw_data_size (u24), zero_pages (u16), stack_size (u24)
    output.extend_from_slice(&(ro_data_size as u32).to_le_bytes()[..3]); // u24
    output.extend_from_slice(&(rw_data_size as u32).to_le_bytes()[..3]); // u24
    output.extend_from_slice(&0u16.to_le_bytes()); // zero_pages = 0
    output.extend_from_slice(&(stack_size as u32).to_le_bytes()[..3]); // u24

    // RO data (padded to ro_data_size)
    output.extend_from_slice(ro_data);
    output.resize(output.len() + (ro_data_size - ro_data.len()), 0);

    // RW data (padded to rw_data_size)
    output.extend_from_slice(rw_data);
    output.resize(output.len() + (rw_data_size - rw_data.len()), 0);

    // Program length (u32) and program data
    output.extend_from_slice(&(program.len() as u32).to_le_bytes());
    output.extend_from_slice(program);

    Ok(output)
}

fn decode_memory_section(data: &[u8]) -> Result<(usize, usize, usize, &[u8]), &'static str> {
    if data.is_empty() || data[0] != SECTION_MEM_CFG {
        return Err("Expected memory config section");
    }
    let cursor = &data[1..];

    // Section length (general integer)
    let (_section_len, rest) = decode_general_integer(cursor)?;
    let cursor = rest;

    // ro_data_size, rw_data_size, stack_size (all general integers)
    let (ro_data_size, rest) = decode_general_integer(cursor)?;
    let (rw_data_size, rest) = decode_general_integer(rest)?;
    let (stack_size, rest) = decode_general_integer(rest)?;

    Ok((ro_data_size as usize, rw_data_size as usize, stack_size as usize, rest))
}

fn decode_generic_section(expected_type: u8, data: &[u8]) -> Result<(&[u8], &[u8]), &'static str> {
    if data.is_empty() || data[0] != expected_type {
        return Err("Unexpected section type");
    }
    let cursor = &data[1..];

    // Section length (general integer)
    let (len, rest) = decode_general_integer(cursor)?;
    let len = len as usize;

    if rest.len() < len {
        return Err("Section data truncated");
    }

    Ok((&rest[..len], &rest[len..]))
}

fn decode_imports_section(data: &[u8]) -> Result<((), &[u8]), &'static str> {
    if data.is_empty() || data[0] != 4 {
        return Err("Expected imports section");
    }
    let cursor = &data[1..];

    // Section length
    let (section_len, rest) = decode_general_integer(cursor)?;
    let section_len = section_len as usize;

    if rest.len() < section_len {
        return Err("Imports section truncated");
    }

    Ok(((), &rest[section_len..]))
}

/// Decode a "general integer" as per GP definition 275.
fn decode_general_integer(data: &[u8]) -> Result<(u64, &[u8]), &'static str> {
    if data.is_empty() {
        return Err("Missing general integer");
    }

    let prefix = data[0];
    let rest = &data[1..];

    if prefix == 0 {
        return Ok((0, rest));
    }

    if prefix < 128 {
        // 0xxxxxxx: value is in the lower 7 bits
        return Ok((prefix as u64, rest));
    }

    if prefix == 0xFF {
        // Full 8 bytes follow
        if rest.len() < 8 {
            return Err("Truncated general integer");
        }
        let value = u64::from_le_bytes(rest[..8].try_into().unwrap());
        return Ok((value, &rest[8..]));
    }

    // Variable length encoding
    let (l, m) = match prefix {
        128..=191 => (1, 128),
        192..=223 => (2, 192),
        224..=239 => (3, 224),
        240..=247 => (4, 240),
        248..=251 => (5, 248),
        252..=253 => (6, 252),
        254 => (7, 254),
        _ => return Err("Invalid general integer prefix"),
    };

    if rest.len() < l {
        return Err("Truncated general integer");
    }

    let m_val = (prefix - m) as u64;
    let mut v: u64 = 0;
    for i in 0..l {
        v |= (rest[i] as u64) << (8 * i);
    }
    v += m_val << (8 * l);

    Ok((v, &rest[l..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_general_integer() {
        assert_eq!(decode_general_integer(&[0]).unwrap(), (0, &[][..]));
        assert_eq!(decode_general_integer(&[1]).unwrap(), (1, &[][..]));
        assert_eq!(decode_general_integer(&[127]).unwrap(), (127, &[][..]));
        assert_eq!(decode_general_integer(&[0x80, 140]).unwrap(), (140, &[][..]));
        assert_eq!(
            decode_general_integer(&[0xFF, 0xF0, 0xDE, 0xBC, 0x9A, 0x78, 0x56, 0x34, 0x12]).unwrap(),
            (0x123456789ABCDEF0, &[][..])
        );
    }
}
