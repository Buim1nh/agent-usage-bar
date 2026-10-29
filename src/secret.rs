//! DPAPI wrappers: data encrypted with `protect` can only be decrypted by the
//! same Windows user on the same machine.

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows::core::w;

pub fn protect(plain: &[u8]) -> Option<Vec<u8>> {
    let input = blob(plain);
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(&input, w!("agent-usage-bar"), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()?;
        Some(take(out))
    }
}

pub fn unprotect(sealed: &[u8]) -> Option<Vec<u8>> {
    let input = blob(sealed);
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()?;
        Some(take(out))
    }
}

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
}

unsafe fn take(b: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let v = unsafe { std::slice::from_raw_parts(b.pbData, b.cbData as usize) }.to_vec();
    unsafe { LocalFree(Some(HLOCAL(b.pbData.cast()))) };
    v
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trips() {
        let sealed = super::protect(b"secret").unwrap();
        assert_ne!(sealed, b"secret");
        assert_eq!(super::unprotect(&sealed).unwrap(), b"secret");
    }
}
