use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, Error, Read, Seek},
};

use byteorder::{LittleEndian, ReadBytesExt};

const BIN_NONE: u8 = b'\x00';
const BIN_STRING: u8 = b'\x01';
const BIN_INT32: u8 = b'\x02';
const BIN_FLOAT32: u8 = b'\x03';
const BIN_POINTER: u8 = b'\x04';
const BIN_WIDESTRING: u8 = b'\x05';
const BIN_COLOR: u8 = b'\x06';
const BIN_UINT64: u8 = b'\x07';
const BIN_END: u8 = b'\x08';
const BIN_INT64: u8 = b'\x0A';
const BIN_END_ALT: u8 = b'\x0B';

const VERSION_28: u32 = 0x7564428;
const VERSION_29: u32 = 0x7564429;

#[derive(Debug)]
pub enum VdfrError {
    UnsupportedVersion(u32),
    InvalidType(u8),
    ReadError(std::io::Error),
}

impl std::error::Error for VdfrError {}

impl std::fmt::Display for VdfrError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            VdfrError::UnsupportedVersion(v) => write!(f, "Invalid version {:#x}", v),
            VdfrError::InvalidType(t) => write!(f, "Invalid type {:#x}", t),
            VdfrError::ReadError(e) => e.fmt(f),
        }
    }
}

impl From<std::io::Error> for VdfrError {
    fn from(e: std::io::Error) -> Self {
        VdfrError::ReadError(e)
    }
}

#[derive(Debug)]
pub enum Value {
    StringType(String),
    WideStringType(String),
    Int32Type(i32),
    PointerType(i32),
    ColorType(i32),
    UInt64Type(u64),
    Int64Type(i64),
    Float32Type(f32),
    KeyValueType(KeyValues),
}

type KeyValues = HashMap<String, Value>;

// Recursively search for the specified sequence of keys in the key-value data.
// The order of the keys dictates the hierarchy, with all except the last having
// to be a Value::KeyValueType.
fn find_keys<'a>(kv: &'a KeyValues, keys: &[&str]) -> Option<&'a Value> {
    if keys.is_empty() {
        return None;
    }

    let key = keys.first().unwrap();
    let value = kv.get(&key.to_string());
    if keys.len() == 1 {
        value
    } else if let Some(Value::KeyValueType(kv)) = value {
        find_keys(kv, &keys[1..])
    } else {
        None
    }
}

#[derive(Debug)]
pub struct App {
    pub size: u32,
    pub state: u32,
    pub last_update: u32,
    pub access_token: u64,
    pub checksum_txt: [u8; 20],
    pub checksum_bin: [u8; 20],
    pub change_number: u32,
    pub key_values: KeyValues,
}

#[derive(Debug)]
pub struct AppInfo {
    pub magic: u32,
    pub universe: u32,
    pub apps: HashMap<u32, App>,
}

impl AppInfo {
    pub fn read(reader: &mut BufReader<File>) -> Result<AppInfo, VdfrError> {
        let magic = reader.read_u32::<LittleEndian>()?;

        if ![VERSION_28, VERSION_29].contains(&magic) {
            return Err(VdfrError::UnsupportedVersion(magic));
        }

        let universe = reader.read_u32::<LittleEndian>()?;

        let string_table = if magic == VERSION_29 {
            Some(AppInfo::read_string_table(reader)?)
        } else {
            None
        };

        let mut appinfo = AppInfo {
            universe,
            magic,
            apps: HashMap::new(),
        };

        loop {
            let app_id = reader.read_u32::<LittleEndian>()?;
            if app_id == 0 {
                break;
            }

            let size = reader.read_u32::<LittleEndian>()?;
            let state = reader.read_u32::<LittleEndian>()?;
            let last_update = reader.read_u32::<LittleEndian>()?;
            let access_token = reader.read_u64::<LittleEndian>()?;

            let mut checksum_txt: [u8; 20] = [0; 20];
            reader.read_exact(&mut checksum_txt)?;

            let change_number = reader.read_u32::<LittleEndian>()?;

            let mut checksum_bin: [u8; 20] = [0; 20];
            reader.read_exact(&mut checksum_bin)?;

            let key_values = read_kv(reader, false, &string_table)?;

            let app = App {
                size,
                state,
                last_update,
                access_token,
                checksum_txt,
                checksum_bin,
                change_number,
                key_values,
            };
            appinfo.apps.insert(app_id, app);
        }

        Ok(appinfo)
    }

    fn read_string_table(reader: &mut BufReader<File>) -> Result<Vec<String>, std::io::Error> {
        let string_table_offset = reader.read_i64::<LittleEndian>()?;
        let original_seek_position = reader.stream_position()?;
        reader.seek(std::io::SeekFrom::Start(string_table_offset as u64))?;
        let num_strings = reader.read_u32::<LittleEndian>()?;
        let mut string_table_bytes: Vec<u8> = Vec::new();
        reader.read_to_end(&mut string_table_bytes)?;
        let string_table: Vec<String> = string_table_bytes
            .split(|&byte| byte == 0)
            .filter(|subslice| !subslice.is_empty()) // Filter out any empty slices (if any)
            .map(|subslice| String::from_utf8_lossy(subslice).into_owned()) // Convert each subslice to a String
            .collect();
        assert!(string_table.len() == num_strings as usize);
        reader.seek(std::io::SeekFrom::Start(original_seek_position))?;

        Ok(string_table)
    }
}

impl App {
    pub fn get(&self, keys: &[&str]) -> Option<&Value> {
        find_keys(&self.key_values, keys)
    }
}

#[derive(Debug)]
pub struct Package {
    pub checksum: [u8; 20],
    pub change_number: u32,
    pub pics: u64,
    pub key_values: KeyValues,
}

#[derive(Debug)]
pub struct PackageInfo {
    pub magic: u32,
    pub universe: u32,
    pub packages: HashMap<u32, Package>,
}

impl PackageInfo {
    pub fn read(reader: &mut BufReader<File>) -> Result<PackageInfo, VdfrError> {
        let magic = reader.read_u32::<LittleEndian>()?;
        let universe = reader.read_u32::<LittleEndian>()?;

        let mut packageinfo = PackageInfo {
            magic,
            universe,
            packages: HashMap::new(),
        };

        loop {
            let package_id = reader.read_u32::<LittleEndian>()?;

            if package_id == 0xffffffff {
                break;
            }

            let mut checksum: [u8; 20] = [0; 20];
            reader.read_exact(&mut checksum)?;

            let change_number = reader.read_u32::<LittleEndian>()?;

            // XXX: No idea what this is. Seems to get ignored in vdf.py.
            let pics = reader.read_u64::<LittleEndian>()?;

            let key_values = read_kv(reader, false, &None)?;

            let package = Package {
                checksum,
                change_number,
                pics,
                key_values,
            };

            packageinfo.packages.insert(package_id, package);
        }

        Ok(packageinfo)
    }
}

impl Package {
    pub fn get(&self, keys: &[&str]) -> Option<&Value> {
        find_keys(&self.key_values, keys)
    }
}

fn read_kv<R: std::io::Read>(
    reader: &mut R,
    alt_format: bool,
    string_table: &Option<Vec<String>>,
) -> Result<KeyValues, VdfrError> {
    let current_bin_end = if alt_format { BIN_END_ALT } else { BIN_END };

    let mut node = KeyValues::new();

    loop {
        let t = reader.read_u8()?;
        if t == current_bin_end {
            return Ok(node);
        }

        let key = if let Some(string_table) = string_table {
            let string_table_index = reader.read_u32::<LittleEndian>()?;
            string_table[string_table_index as usize].clone()
        } else {
            read_string(reader, false)?
        };

        if t == BIN_NONE {
            let subnode = read_kv(reader, alt_format, string_table)?;
            node.insert(key, Value::KeyValueType(subnode));
        } else if t == BIN_STRING {
            let s = read_string(reader, false)?;
            node.insert(key, Value::StringType(s));
        } else if t == BIN_WIDESTRING {
            let s = read_string(reader, true)?;
            node.insert(key, Value::WideStringType(s));
        } else if [BIN_INT32, BIN_POINTER, BIN_COLOR].contains(&t) {
            let val = reader.read_i32::<LittleEndian>()?;
            if t == BIN_INT32 {
                node.insert(key, Value::Int32Type(val));
            } else if t == BIN_POINTER {
                node.insert(key, Value::PointerType(val));
            } else if t == BIN_COLOR {
                node.insert(key, Value::ColorType(val));
            }
        } else if t == BIN_UINT64 {
            let val = reader.read_u64::<LittleEndian>()?;
            node.insert(key, Value::UInt64Type(val));
        } else if t == BIN_INT64 {
            let val = reader.read_i64::<LittleEndian>()?;
            node.insert(key, Value::Int64Type(val));
        } else if t == BIN_FLOAT32 {
            let val = reader.read_f32::<LittleEndian>()?;
            node.insert(key, Value::Float32Type(val));
        } else {
            return Err(VdfrError::InvalidType(t));
        }
    }
}

fn read_string<R: std::io::Read>(reader: &mut R, wide: bool) -> Result<String, Error> {
    if wide {
        let mut buf: Vec<u16> = vec![];
        loop {
            // Maybe this should be big-endian?
            let c = reader.read_u16::<LittleEndian>()?;
            if c == 0 {
                break;
            }
            buf.push(c);
        }
        Ok(std::string::String::from_utf16_lossy(&buf).to_string())
    } else {
        let mut buf: Vec<u8> = vec![];
        loop {
            let c = reader.read_u8()?;
            if c == 0 {
                break;
            }
            buf.push(c);
        }
        Ok(std::string::String::from_utf8_lossy(&buf).to_string())
    }
}
