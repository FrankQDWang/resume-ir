use std::fs;
use std::path::Path;

const MAX_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;

const DOS_PE_OFFSET: usize = 0x3c;
const PE_SIGNATURE_BYTES: usize = 4;
const COFF_HEADER_BYTES: usize = 20;
const PE32_PLUS_MAGIC: u16 = 0x20b;
const AMD64_MACHINE: u16 = 0x8664;
const CHECKSUM_OFFSET: usize = 64;
const DATA_DIRECTORY_COUNT_OFFSET: usize = 108;
const DATA_DIRECTORY_OFFSET: usize = 112;
const SECURITY_DIRECTORY_INDEX: usize = 4;
const DATA_DIRECTORY_ENTRY_BYTES: usize = 8;

pub(super) struct CanonicalPayload {
    pub(super) architecture: &'static str,
    pub(super) bytes: Vec<u8>,
}

pub(super) fn read_canonical_payload(path: &Path) -> Result<CanonicalPayload, ()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_EXECUTABLE_BYTES
    {
        return Err(());
    }
    canonical_payload(fs::read(path).map_err(|_| ())?)
}

fn canonical_payload(mut bytes: Vec<u8>) -> Result<CanonicalPayload, ()> {
    if bytes.len() < DOS_PE_OFFSET + 4 || bytes.get(0..2) != Some(b"MZ") {
        return Err(());
    }
    let pe_offset = read_u32(&bytes, DOS_PE_OFFSET)? as usize;
    let coff_offset = pe_offset.checked_add(PE_SIGNATURE_BYTES).ok_or(())?;
    let optional_offset = coff_offset.checked_add(COFF_HEADER_BYTES).ok_or(())?;
    if bytes.get(pe_offset..coff_offset) != Some(b"PE\0\0")
        || read_u16(&bytes, coff_offset)? != AMD64_MACHINE
        || read_u16(&bytes, optional_offset)? != PE32_PLUS_MAGIC
    {
        return Err(());
    }
    let optional_bytes = read_u16(&bytes, coff_offset + 16)? as usize;
    let optional_end = optional_offset.checked_add(optional_bytes).ok_or(())?;
    let checksum_offset = optional_offset.checked_add(CHECKSUM_OFFSET).ok_or(())?;
    let directory_count_offset = optional_offset
        .checked_add(DATA_DIRECTORY_COUNT_OFFSET)
        .ok_or(())?;
    let security_entry_offset = optional_offset
        .checked_add(DATA_DIRECTORY_OFFSET)
        .and_then(|offset| {
            offset.checked_add(SECURITY_DIRECTORY_INDEX * DATA_DIRECTORY_ENTRY_BYTES)
        })
        .ok_or(())?;
    if optional_end > bytes.len()
        || checksum_offset + 4 > optional_end
        || directory_count_offset + 4 > optional_end
        || read_u32(&bytes, directory_count_offset)? as usize <= SECURITY_DIRECTORY_INDEX
        || security_entry_offset + DATA_DIRECTORY_ENTRY_BYTES > optional_end
    {
        return Err(());
    }
    bytes[checksum_offset..checksum_offset + 4].fill(0);
    let certificate_offset = read_u32(&bytes, security_entry_offset)? as usize;
    let certificate_bytes = read_u32(&bytes, security_entry_offset + 4)? as usize;
    bytes[security_entry_offset..security_entry_offset + DATA_DIRECTORY_ENTRY_BYTES].fill(0);
    if certificate_offset == 0 && certificate_bytes == 0 {
        return Ok(CanonicalPayload {
            architecture: "x86_64",
            bytes,
        });
    }
    let certificate_end = certificate_offset
        .checked_add(certificate_bytes)
        .ok_or(())?;
    if certificate_offset < optional_end || certificate_bytes == 0 || certificate_end != bytes.len()
    {
        return Err(());
    }
    bytes.truncate(certificate_offset);
    Ok(CanonicalPayload {
        architecture: "x86_64",
        bytes,
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, ()> {
    let value: [u8; 2] = bytes
        .get(offset..offset + 2)
        .ok_or(())?
        .try_into()
        .map_err(|_| ())?;
    Ok(u16::from_le_bytes(value))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ()> {
    let value: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or(())?
        .try_into()
        .map_err(|_| ())?;
    Ok(u32::from_le_bytes(value))
}
