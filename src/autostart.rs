use std::io;

use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "lolPing";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Points the Run key at this exe with `--hidden`, or removes the entry.
pub fn set(enabled: bool) -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let key_path = wide(RUN_KEY);
    let value = wide(VALUE);
    unsafe {
        let mut key = HKEY::default();
        RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(key_path.as_ptr()), Some(0), KEY_SET_VALUE, &mut key)
            .ok()
            .map_err(|e| io::Error::other(e.message()))?;
        let result = if enabled {
            let command = wide(&format!("\"{}\" --hidden", exe.display()));
            let bytes: Vec<u8> = command.iter().flat_map(|c| c.to_le_bytes()).collect();
            RegSetValueExW(key, PCWSTR(value.as_ptr()), None, REG_SZ, Some(&bytes)).ok()
        } else {
            let _ = RegDeleteValueW(key, PCWSTR(value.as_ptr()));
            Ok(())
        };
        let _ = RegCloseKey(key);
        result.map_err(|e| io::Error::other(e.message()))
    }
}
