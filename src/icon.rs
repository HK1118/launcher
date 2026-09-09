#![allow(clippy::upper_case_acronyms)]

use eframe::egui;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HBITMAP, HDC, HGDIOBJ, ReleaseDC,
};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM};
use windows::Win32::UI::Controls::{HIMAGELIST, ILD_NORMAL, ImageList_GetIcon};
use windows::Win32::UI::Shell::{
    IShellLinkW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SYSICONINDEX, SHGetFileInfoW,
    ShellLink,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};
use windows::core::{Interface, PCWSTR};

// --- RAII リソースガード（リーク防止用） ---
struct IconGuard(HICON);
impl Drop for IconGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DestroyIcon(self.0);
            }
        }
    }
}

struct BitmapGuard(HBITMAP);
impl Drop for BitmapGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.0.0));
            }
        }
    }
}

struct DcGuard {
    hdc: HDC,
}
impl Drop for DcGuard {
    fn drop(&mut self) {
        if !self.hdc.is_invalid() {
            unsafe {
                ReleaseDC(None, self.hdc);
            }
        }
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn resize_rgba(src: &[u8], src_w: usize, src_h: usize, dst_w: usize, dst_h: usize) -> Vec<u8> {
    let mut dst = vec![0u8; dst_w * dst_h * 4];
    for y in 0..dst_h {
        let src_y = (y as f32 + 0.5) * (src_h as f32 / dst_h as f32) - 0.5;
        let y0 = (src_y.floor() as isize).clamp(0, src_h as isize - 1) as usize;
        let y1 = (y0 + 1).min(src_h - 1);
        let fy = src_y - src_y.floor();

        for x in 0..dst_w {
            let src_x = (x as f32 + 0.5) * (src_w as f32 / dst_w as f32) - 0.5;
            let x0 = (src_x.floor() as isize).clamp(0, src_w as isize - 1) as usize;
            let x1 = (x0 + 1).min(src_w - 1);
            let fx = src_x - src_x.floor();

            let idx00 = (y0 * src_w + x0) * 4;
            let idx10 = (y0 * src_w + x1) * 4;
            let idx01 = (y1 * src_w + x0) * 4;
            let idx11 = (y1 * src_w + x1) * 4;

            let dst_idx = (y * dst_w + x) * 4;

            for c in 0..4 {
                let top = (1.0 - fx) * src[idx00 + c] as f32 + fx * src[idx10 + c] as f32;
                let bottom = (1.0 - fx) * src[idx01 + c] as f32 + fx * src[idx11 + c] as f32;
                let val = (1.0 - fy) * top + fy * bottom;
                dst[dst_idx + c] = val.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    dst
}

/// Steamなどの .url ファイルから指定されているアイコンファイルのパスを取り出す
fn get_url_icon_path(path: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix("IconFile=") {
            let p = val.trim().to_string();
            if Path::new(&p).exists() {
                return Some(p);
            }
        }
    }
    None
}

/// VALORANT等の .lnk ファイルからアイコンパスまたはリンク先を取得
fn get_lnk_icon_path(path: &str) -> Option<String> {
    unsafe {
        let shell_link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let persist_file: IPersistFile = shell_link.cast().ok()?;

        let wide_path = to_wide(path);
        persist_file
            .Load(PCWSTR(wide_path.as_ptr()), STGM(0))
            .ok()?;

        let mut icon_buf = [0u16; 1024];
        let mut icon_idx = 0i32;

        if shell_link
            .GetIconLocation(&mut icon_buf, &mut icon_idx)
            .is_ok()
        {
            let len = icon_buf.iter().position(|&c| c == 0).unwrap_or(0);
            let icon_str = String::from_utf16_lossy(&icon_buf[..len]);
            if !icon_str.is_empty() && Path::new(&icon_str).exists() {
                return Some(icon_str);
            }
        }

        let mut target_buf = [0u16; 1024];
        if shell_link
            .GetPath(&mut target_buf, std::ptr::null_mut(), 0)
            .is_ok()
        {
            let len = target_buf.iter().position(|&c| c == 0).unwrap_or(0);
            let target_str = String::from_utf16_lossy(&target_buf[..len]);
            if !target_str.is_empty() && Path::new(&target_str).exists() {
                return Some(target_str);
            }
        }

        None
    }
}

pub unsafe fn hicon_to_color_image(h_icon: HICON) -> Option<egui::ColorImage> {
    // 関数を抜けた時に確実にアイコンを破棄
    let _icon_guard = IconGuard(h_icon);

    unsafe {
        let mut icon_info = ICONINFO::default();
        if GetIconInfo(h_icon, &mut icon_info).is_err() {
            return None;
        }

        // ビットマップハンドルの RAII ガード
        let _color_guard = BitmapGuard(icon_info.hbmColor);
        let _mask_guard = BitmapGuard(icon_info.hbmMask);

        let target_bitmap = if !icon_info.hbmColor.is_invalid() {
            icon_info.hbmColor
        } else {
            icon_info.hbmMask
        };

        let mut bm = BITMAP::default();
        if GetObjectW(
            HGDIOBJ(target_bitmap.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut std::ffi::c_void),
        ) == 0
        {
            return None;
        }

        let orig_w = bm.bmWidth.max(1);
        let orig_h = if icon_info.hbmColor.is_invalid() {
            (bm.bmHeight / 2).max(1)
        } else {
            bm.bmHeight.max(1)
        };

        let hdc = GetDC(None);
        if hdc.is_invalid() {
            return None;
        }
        let _dc_guard = DcGuard { hdc };

        let mut pixels = vec![0u8; (orig_w * orig_h * 4) as usize];
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: orig_w,
                biHeight: -orig_h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [windows::Win32::Graphics::Gdi::RGBQUAD::default()],
        };

        let lines = GetDIBits(
            hdc,
            target_bitmap,
            0,
            orig_h as u32,
            Some(pixels.as_mut_ptr() as *mut std::ffi::c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        if lines == 0 {
            return None;
        }

        let (chunks, _) = pixels.as_chunks_mut::<4>();
        for chunk in chunks.iter_mut() {
            chunk.swap(0, 2);
        }

        let has_alpha = chunks.iter().any(|c| c[3] > 0);
        if !has_alpha {
            for chunk in chunks.iter_mut() {
                chunk[3] = 255;
            }
        }

        let target_size = 32;
        let final_pixels = if orig_w as usize > target_size || orig_h as usize > target_size {
            resize_rgba(
                &pixels,
                orig_w as usize,
                orig_h as usize,
                target_size,
                target_size,
            )
        } else {
            pixels
        };

        let (final_w, final_h) = if orig_w as usize > target_size || orig_h as usize > target_size {
            (target_size, target_size)
        } else {
            (orig_w as usize, orig_h as usize)
        };

        Some(egui::ColorImage::from_rgba_unmultiplied(
            [final_w, final_h],
            &final_pixels,
        ))
    }
}

pub fn extract_icon_image(path: &str) -> Option<egui::ColorImage> {
    let lower = path.to_lowercase();

    // 1. Steamなどの .url ファイルなら中身の .ico パスを直接抽出
    if lower.ends_with(".url")
        && let Some(icon_path) = get_url_icon_path(path)
        && let Some(img) = extract_icon_image(&icon_path)
    {
        return Some(img);
    }

    // 2. VALORANTなどの .lnk ならショートカットのカスタムアイコンまたはリンク先の実体を取得
    if lower.ends_with(".lnk")
        && let Some(target_path) = get_lnk_icon_path(path)
        && let Some(img) = extract_icon_image(&target_path)
    {
        return Some(img);
    }

    let wide_path = to_wide(path);

    // 3. 通常の矢印なしアイコン取得（exeなど）
    unsafe {
        let mut shfi = SHFILEINFOW::default();

        let himl = SHGetFileInfoW(
            PCWSTR(wide_path.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX | SHGFI_LARGEICON,
        );

        if himl != 0 {
            let h_icon = ImageList_GetIcon(HIMAGELIST(himl as isize), shfi.iIcon, ILD_NORMAL);
            if !h_icon.is_invalid()
                && let Some(img) = hicon_to_color_image(h_icon)
            {
                return Some(img);
            }
        }

        // 4. フォールバック: ショートカットそのまま（矢印付き）で確実に取得
        let mut shfi_direct = SHFILEINFOW::default();

        let res = SHGetFileInfoW(
            PCWSTR(wide_path.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut shfi_direct),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );

        if res != 0 && !shfi_direct.hIcon.is_invalid() {
            return hicon_to_color_image(shfi_direct.hIcon);
        }
    }

    None
}
