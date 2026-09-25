//! Real Windows Shell icons for indexed Desktop entries.
//!
//! The UI must show the same image Explorer shows, so icons are extracted with
//! `SHGetFileInfoW`, rendered into a top-down 32bpp DIB, encoded as PNG, and
//! returned as a `data:` URL. This module only reads icons; it never touches
//! file contents or the filesystem mutation boundary.

use crate::core::CoreResult;
use base64::Engine;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// Matches the list density of the file panel.
#[cfg(windows)]
const ICON_SIZE: i32 = 16;

/// A batch entry from the UI. `key` is the UI-side cache identity (an
/// extension for shared icons, a full path for files with embedded icons).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconRequest {
    pub key: String,
    pub path: String,
    pub is_dir: bool,
}

/// Icons are immutable per key for the lifetime of the process, so the cache
/// never needs invalidation.
fn cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolves every request, returning only the keys that produced an icon.
/// Unknown or unreadable paths are skipped instead of failing the whole batch.
pub fn resolve(requests: Vec<IconRequest>) -> CoreResult<HashMap<String, String>> {
    let mut resolved = HashMap::new();
    let mut missing = Vec::new();
    {
        let cached = cache()
            .lock()
            .map_err(|_| crate::core::CoreError::Invalid("icon cache lock poisoned".to_string()))?;
        for request in requests {
            match cached.get(&request.key) {
                Some(data_url) => {
                    resolved.insert(request.key.clone(), data_url.clone());
                }
                None => missing.push(request),
            }
        }
    }
    if missing.is_empty() {
        return Ok(resolved);
    }
    let mut fresh = Vec::new();
    for request in missing {
        if let Some(data_url) = icon_data_url(Path::new(&request.path), request.is_dir) {
            fresh.push((request.key, data_url));
        }
    }
    if !fresh.is_empty() {
        if let Ok(mut cached) = cache().lock() {
            for (key, data_url) in &fresh {
                cached.insert(key.clone(), data_url.clone());
            }
        }
        resolved.extend(fresh);
    }
    Ok(resolved)
}

/// Returns a PNG `data:` URL for one path, or `None` when Windows cannot
/// produce an icon for it.
pub fn icon_data_url(path: &Path, is_dir: bool) -> Option<String> {
    let rgba = render_rgba(path, is_dir, ICON_SIZE)?;
    let png = encode_png(&rgba, ICON_SIZE as u32, ICON_SIZE as u32).ok()?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
    Some(format!("data:image/png;base64,{encoded}"))
}

#[cfg(windows)]
fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, png::EncodingError> {
    let mut buffer = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buffer, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgba)?;
    }
    Ok(buffer)
}

/// `SHGetFileInfoW` drives shell state that is not safe to query from several
/// threads at once; concurrent calls fail intermittently. Extraction is
/// therefore serialized. Because results are cached per key, the lock is only
/// contended for the first request of each icon.
#[cfg(windows)]
fn shell_guard() -> std::sync::MutexGuard<'static, ()> {
    static SHELL: OnceLock<Mutex<()>> = OnceLock::new();
    SHELL
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

#[cfg(windows)]
fn render_rgba(path: &Path, is_dir: bool, size: i32) -> Option<Vec<u8>> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::DestroyIcon;

    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

    let _guard = shell_guard();
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let attributes = if is_dir {
        FILE_ATTRIBUTE_DIRECTORY
    } else {
        FILE_ATTRIBUTE_NORMAL
    };
    let base = SHGFI_ICON | SHGFI_SMALLICON;
    // A real lookup returns the icon embedded in the file, which is what makes
    // executables and shortcuts match Explorer. `SHGFI_USEFILEATTRIBUTES`
    // resolves from the extension alone and is used as a fallback so a locked,
    // renamed, or just-deleted entry still shows its type icon.
    for flags in [base, base | SHGFI_USEFILEATTRIBUTES] {
        let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
        let result = unsafe {
            SHGetFileInfoW(
                wide.as_ptr(),
                attributes,
                &mut info,
                std::mem::size_of::<SHFILEINFOW>() as u32,
                flags,
            )
        };
        if result == 0 || info.hIcon.is_null() {
            continue;
        }
        let rgba = unsafe { draw_icon(info.hIcon, size) };
        unsafe { DestroyIcon(info.hIcon) };
        if rgba.is_some() {
            return rgba;
        }
    }
    None
}

#[cfg(windows)]
unsafe fn draw_icon(
    icon: windows_sys::Win32::UI::WindowsAndMessaging::HICON,
    size: i32,
) -> Option<Vec<u8>> {
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{DrawIconEx, DI_NORMAL};

    let dc = CreateCompatibleDC(std::ptr::null_mut());
    if dc.is_null() {
        return None;
    }
    let mut info: BITMAPINFO = std::mem::zeroed();
    info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = size;
    // Negative height requests a top-down bitmap, matching PNG row order.
    info.bmiHeader.biHeight = -size;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let bitmap = CreateDIBSection(
        dc,
        &info,
        DIB_RGB_COLORS,
        &mut bits,
        std::ptr::null_mut(),
        0,
    );
    if bitmap.is_null() || bits.is_null() {
        DeleteDC(dc);
        return None;
    }
    let previous = SelectObject(dc, bitmap as *mut std::ffi::c_void);
    let drawn = DrawIconEx(
        dc,
        0,
        0,
        icon,
        size,
        size,
        0,
        std::ptr::null_mut(),
        DI_NORMAL,
    );
    let byte_count = (size * size * 4) as usize;
    let mut rgba = vec![0u8; byte_count];
    if drawn != 0 {
        let source = std::slice::from_raw_parts(bits as *const u8, byte_count);
        let mut max_alpha = 0u8;
        for index in 0..(size * size) as usize {
            let offset = index * 4;
            // DIB is BGRA; PNG needs RGBA.
            rgba[offset] = source[offset + 2];
            rgba[offset + 1] = source[offset + 1];
            rgba[offset + 2] = source[offset];
            rgba[offset + 3] = source[offset + 3];
            max_alpha = max_alpha.max(source[offset + 3]);
        }
        if max_alpha == 0 {
            // The icon carried no alpha channel and the mask was consumed by
            // DrawIconEx, so the result is an opaque square. Shell icons on
            // Windows 10/11 always have alpha; this only guards legacy icons.
            for index in 0..(size * size) as usize {
                rgba[index * 4 + 3] = 255;
            }
        } else {
            unpremultiply_if_needed(&mut rgba);
        }
    }
    SelectObject(dc, previous);
    DeleteObject(bitmap as *mut std::ffi::c_void);
    DeleteDC(dc);
    Some(rgba)
}

/// `DrawIconEx` composites premultiplied alpha, while PNG stores straight
/// alpha. Premultiplied data can never exceed its alpha, so any channel above
/// alpha proves the buffer is already straight and must be left alone.
#[cfg(windows)]
fn unpremultiply_if_needed(rgba: &mut [u8]) {
    let straight = rgba
        .chunks_exact(4)
        .any(|pixel| pixel[0] > pixel[3] || pixel[1] > pixel[3] || pixel[2] > pixel[3]);
    if straight {
        return;
    }
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        if alpha == 0 || alpha == 255 {
            continue;
        }
        for channel in 0..3 {
            let value = (pixel[channel] as u32 * 255 + alpha / 2) / alpha;
            pixel[channel] = value.min(255) as u8;
        }
    }
}

#[cfg(not(windows))]
fn encode_png(_rgba: &[u8], _width: u32, _height: u32) -> Result<Vec<u8>, ()> {
    Err(())
}

#[cfg(not(windows))]
fn render_rgba(_path: &Path, _is_dir: bool, _size: i32) -> Option<Vec<u8>> {
    // Icon extraction uses the Windows Shell; other platforms fall back to the
    // neutral placeholder the UI renders when no icon is returned.
    None
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn icons_render_to_png_data_urls_and_are_cached() {
        let temp = tempfile::tempdir().unwrap();
        let desktop = temp.path().join("Desktop");
        std::fs::create_dir(&desktop).unwrap();
        let file = desktop.join("report.txt");
        std::fs::write(&file, b"report").unwrap();

        let data_url = icon_data_url(&file, false).expect("text icon");
        assert!(data_url.starts_with("data:image/png;base64,"));
        let folder_url = icon_data_url(&desktop, true).expect("folder icon");
        assert!(folder_url.starts_with("data:image/png;base64,"));
        // A folder and a text file must not share the same image.
        assert_ne!(data_url, folder_url);

        let resolved = resolve(vec![
            IconRequest {
                key: "ext:txt".into(),
                path: file.to_string_lossy().into_owned(),
                is_dir: false,
            },
            IconRequest {
                key: "__dir__".into(),
                path: desktop.to_string_lossy().into_owned(),
                is_dir: true,
            },
        ])
        .unwrap();
        assert_eq!(resolved.len(), 2);
        // Second pass is served from the cache.
        let again = resolve(vec![IconRequest {
            key: "ext:txt".into(),
            path: file.to_string_lossy().into_owned(),
            is_dir: false,
        }])
        .unwrap();
        assert_eq!(again.get("ext:txt"), resolved.get("ext:txt"));
    }

    #[test]
    fn common_extensions_resolve_to_png_icons() {
        let temp = tempfile::tempdir().unwrap();
        for extension in ["txt", "pdf", "png", "zip", "mp4", "docx", "rs", "json"] {
            let key = format!("ext:{extension}");
            let path = temp.path().join(format!("sample.{extension}"));
            std::fs::write(&path, b"sample").unwrap();
            let resolved = resolve(vec![IconRequest {
                key: key.clone(),
                path: path.to_string_lossy().into_owned(),
                is_dir: false,
            }])
            .unwrap();
            let icon = resolved
                .get(&key)
                .unwrap_or_else(|| panic!("{extension} produced no icon"));
            assert!(icon.starts_with("data:image/png;base64,"), "{extension}");
        }
    }

    #[test]
    fn unknown_extensions_never_fail_the_batch() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nope.zzz");
        std::fs::write(&path, b"x").unwrap();
        // Windows has no association for `.zzz`; the batch must still succeed
        // and simply omit the key so the UI shows its placeholder.
        let resolved = resolve(vec![IconRequest {
            key: "ext:zzz".into(),
            path: path.to_string_lossy().into_owned(),
            is_dir: false,
        }])
        .unwrap();
        assert!(resolved.len() <= 1);
    }
}
