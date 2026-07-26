use std::fs;
use std::io::Read;
use std::path::Path;

pub(crate) const MAX_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;

const MAGIC_64_LE: u32 = 0xfeedfacf;
const CPU_ARM64: u32 = 0x0100000c;
const LC_CODE_SIGNATURE: u32 = 0x1d;
const LC_SEGMENT_64: u32 = 0x19;
const SEGMENT_COMMAND_BYTES: usize = 72;
const SECTION_64_BYTES: usize = 80;
const MAX_LOAD_COMMANDS: u32 = 4096;

pub(crate) struct CanonicalPayload {
    pub(crate) architecture: &'static str,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) fn read_canonical_payload(path: &Path) -> Result<CanonicalPayload, ()> {
    let metadata = fs::metadata(path).map_err(|_| ())?;
    if metadata.len() == 0 || metadata.len() > MAX_EXECUTABLE_BYTES {
        return Err(());
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).map_err(|_| ())?);
    fs::File::open(path)
        .map_err(|_| ())?
        .take(MAX_EXECUTABLE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() as u64 != metadata.len() {
        return Err(());
    }
    canonical_payload(bytes)
}

fn canonical_payload(bytes: Vec<u8>) -> Result<CanonicalPayload, ()> {
    if bytes.len() < 32 || read_u32(&bytes, 0)? != MAGIC_64_LE || read_u32(&bytes, 4)? != CPU_ARM64
    {
        return Err(());
    }
    let command_count = read_u32(&bytes, 16)?;
    let command_bytes = usize::try_from(read_u32(&bytes, 20)?).map_err(|_| ())?;
    let command_end = 32_usize
        .checked_add(command_bytes)
        .filter(|end| *end <= bytes.len())
        .ok_or(())?;
    if command_count > MAX_LOAD_COMMANDS
        || usize::try_from(command_count)
            .ok()
            .and_then(|count| count.checked_mul(8))
            .is_none_or(|minimum| minimum > command_bytes)
    {
        return Err(());
    }

    let mut offset = 32_usize;
    let mut signature = None;
    let mut linkedit_command = None;
    for _ in 0..command_count {
        if offset.checked_add(8).is_none_or(|end| end > command_end) {
            return Err(());
        }
        let command = read_u32(&bytes, offset)?;
        let size = usize::try_from(read_u32(&bytes, offset + 4)?).map_err(|_| ())?;
        if size < 8 || size % 8 != 0 || offset.checked_add(size).is_none_or(|end| end > command_end)
        {
            return Err(());
        }
        if command == LC_CODE_SIGNATURE {
            if signature.is_some() || size != 16 {
                return Err(());
            }
            signature = Some((
                offset,
                usize::try_from(read_u32(&bytes, offset + 8)?).map_err(|_| ())?,
                usize::try_from(read_u32(&bytes, offset + 12)?).map_err(|_| ())?,
            ));
        }
        if command == LC_SEGMENT_64 {
            validate_segment_command(&bytes, offset, size)?;
            if segment_name(&bytes, offset)? == b"__LINKEDIT"
                && linkedit_command.replace(offset).is_some()
            {
                return Err(());
            }
        }
        offset += size;
    }
    if offset != command_end {
        return Err(());
    }

    let canonical = match signature {
        None => bytes,
        Some((command_offset, data_offset, data_size)) => {
            let linkedit_command = linkedit_command.ok_or(())?;
            if data_size == 0
                || data_offset < command_end
                || data_offset
                    .checked_add(data_size)
                    .is_none_or(|end| end != bytes.len())
            {
                return Err(());
            }
            let mut canonical = bytes[..data_offset].to_vec();
            zero(&mut canonical, command_offset + 8, 4)?;
            zero(&mut canonical, command_offset + 12, 4)?;
            zero(&mut canonical, linkedit_command + 32, 8)?;
            zero(&mut canonical, linkedit_command + 48, 8)?;
            canonical
        }
    };
    Ok(CanonicalPayload {
        architecture: "arm64",
        bytes: canonical,
    })
}

fn validate_segment_command(bytes: &[u8], offset: usize, size: usize) -> Result<(), ()> {
    if size < SEGMENT_COMMAND_BYTES {
        return Err(());
    }
    let section_count = usize::try_from(read_u32(bytes, offset + 64)?).map_err(|_| ())?;
    let expected_size = section_count
        .checked_mul(SECTION_64_BYTES)
        .and_then(|sections| SEGMENT_COMMAND_BYTES.checked_add(sections))
        .ok_or(())?;
    if size != expected_size {
        return Err(());
    }
    Ok(())
}

fn segment_name(bytes: &[u8], offset: usize) -> Result<&[u8], ()> {
    let name = bytes.get(offset + 8..offset + 24).ok_or(())?;
    let length = name
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(name.len());
    if name[length..].iter().any(|byte| *byte != 0) {
        return Err(());
    }
    Ok(&name[..length])
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ()> {
    let value: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or(())?
        .try_into()
        .map_err(|_| ())?;
    Ok(u32::from_le_bytes(value))
}

fn zero(bytes: &mut [u8], offset: usize, width: usize) -> Result<(), ()> {
    bytes.get_mut(offset..offset + width).ok_or(())?.fill(0);
    Ok(())
}
