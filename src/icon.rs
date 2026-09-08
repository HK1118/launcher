#![allow(clippy::upper_case_acronyms)]

use eframe::egui;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

#[repr(C)]
struct SHFILEINFOW {
    h_icon: *mut std::ffi::c_void,
    i_icon: i32,
    dw_attributes: u32,
    sz_display_name: [u16; 260],
    sz_type_name: [u16; 80],
}

const SHGFI_ICON: u32 = 0x000000100;
const SHGFI_SYSICONINDEX: u32 = 0x000004000;
const SHGFI_LARGEICON: u32 = 0x000000000;
const ILD_NORMAL: u32 = 0x00000000;

#[repr(C)]
struct ICONINFO {
    f_icon: i32,
    x_hotspot: u32,
    y_hotspot: u32,
    hbm_mask: *mut std::ffi::c_void,
    hbm_color: *mut std::ffi::c_void,
}

#[repr(C)]
struct BITMAP {
    bm_type: i32,
    bm_width: i32,
    bm_height: i32,
    bm_width_bytes: i32,
    bm_planes: u16,
    bm_bits_pixel: u16,
    bm_bits: *mut std::ffi::c_void,
}

#[repr(C)]
struct BITMAPINFOHEADER {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_size_image: u32,
    bi_x_pels_per_meter: i32,
    bi_y_pels_per_meter: i32,
    bi_clr_used: u32,
    bi_clr_important: u32,
}

#[repr(C)]
struct BITMAPINFO {
    bmi_header: BITMAPINFOHEADER,
    bmi_colors: [u32; 1],
}

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const CLSID_SHELL_LINK: Guid = Guid {
    data1: 0x00021401,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

const IID_ISHELL_LINK_W: Guid = Guid {
    data1: 0x000214F9,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

const IID_IPERSIST_FILE: Guid = Guid {
    data1: 0x0000010B,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

#[repr(C)]
struct IShellLinkWVtbl {
    _qi: usize,
    _add_ref: usize,
    release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
    get_path: unsafe extern "system" fn(
        *mut std::ffi::c_void,
        *mut u16,
        i32,
        *mut std::ffi::c_void,
        u32,
    ) -> i32,
    _pad1: [usize; 12],
    get_icon_location:
        unsafe extern "system" fn(*mut std::ffi::c_void, *mut u16, i32, *mut i32) -> i32,
}

#[repr(C)]
struct IPersistFileVtbl {
    _qi: usize,
    _add_ref: usize,
    release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
    _get_class_id: usize,
    _is_dirty: usize,
    load: unsafe extern "system" fn(*mut std::ffi::c_void, *const u16, u32) -> i32,
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoCreateInstance(
        rclsid: *const Guid,
        pUnkOuter: *mut std::ffi::c_void,
        dwClsContext: u32,
        riid: *const Guid,
        ppv: *mut *mut std::ffi::c_void,
    ) -> i32;
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHGetFileInfoW(
        pszPath: *const u16,
        dwFileAttributes: u32,
        psfi: *mut SHFILEINFOW,
        cbFileInfo: u32,
        uFlags: u32,
    ) -> usize;
}

#[link(name = "comctl32")]
unsafe extern "system" {
    fn ImageList_GetIcon(himl: *mut std::ffi::c_void, i: i32, flags: u32) -> *mut std::ffi::c_void;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetIconInfo(hIcon: *mut std::ffi::c_void, piconinfo: *mut ICONINFO) -> i32;
    fn DestroyIcon(hIcon: *mut std::ffi::c_void) -> i32;
    fn GetDC(hWnd: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn ReleaseDC(hWnd: *mut std::ffi::c_void, hDC: *mut std::ffi::c_void) -> i32;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn GetObjectW(h: *mut std::ffi::c_void, c: i32, pv: *mut std::ffi::c_void) -> i32;
    fn GetDIBits(
        hdc: *mut std::ffi::c_void,
        hbm: *mut std::ffi::c_void,
        start: u32,
        cLines: u32,
        lpvBits: *mut std::ffi::c_void,
        lpbmi: *mut BITMAPINFO,
        usage: u32,
    ) -> i32;
    fn DeleteObject(ho: *mut std::ffi::c_void) -> i32;
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
        let mut shell_link: *mut std::ffi::c_void = std::ptr::null_mut();
        if CoCreateInstance(
            &CLSID_SHELL_LINK,
            std::ptr::null_mut(),
            1,
            &IID_ISHELL_LINK_W,
            &mut shell_link,
        ) < 0
            || shell_link.is_null()
        {
            return None;
        }

        let sl_vtbl = *(shell_link as *mut *const IShellLinkWVtbl);
        let mut persist_file: *mut std::ffi::c_void = std::ptr::null_mut();
        // QueryInterface
        let hr = {
            let qi = std::mem::transmute::<
                usize,
                unsafe extern "system" fn(
                    *mut std::ffi::c_void,
                    *const Guid,
                    *mut *mut std::ffi::c_void,
                ) -> i32,
            >((*sl_vtbl)._qi);
            qi(shell_link, &IID_IPERSIST_FILE, &mut persist_file)
        };

        if hr < 0 || persist_file.is_null() {
            ((*sl_vtbl).release)(shell_link);
            return None;
        }

        let pf_vtbl = *(persist_file as *mut *const IPersistFileVtbl);
        let wide_path = to_wide(path);
        if ((*pf_vtbl).load)(persist_file, wide_path.as_ptr(), 0) < 0 {
            ((*pf_vtbl).release)(persist_file);
            ((*sl_vtbl).release)(shell_link);
            return None;
        }

        let mut icon_buf = [0u16; 1024];
        let mut icon_idx = 0i32;
        let mut result = None;

        if ((*sl_vtbl).get_icon_location)(shell_link, icon_buf.as_mut_ptr(), 1024, &mut icon_idx)
            >= 0
        {
            let len = icon_buf.iter().position(|&c| c == 0).unwrap_or(0);
            let icon_str = String::from_utf16_lossy(&icon_buf[..len]);
            if !icon_str.is_empty() && Path::new(&icon_str).exists() {
                result = Some(icon_str);
            }
        }

        if result.is_none() {
            let mut target_buf = [0u16; 1024];
            if ((*sl_vtbl).get_path)(
                shell_link,
                target_buf.as_mut_ptr(),
                1024,
                std::ptr::null_mut(),
                0,
            ) >= 0
            {
                let len = target_buf.iter().position(|&c| c == 0).unwrap_or(0);
                let target_str = String::from_utf16_lossy(&target_buf[..len]);
                if !target_str.is_empty() && Path::new(&target_str).exists() {
                    result = Some(target_str);
                }
            }
        }

        ((*pf_vtbl).release)(persist_file);
        ((*sl_vtbl).release)(shell_link);
        result
    }
}

unsafe fn hicon_to_color_image(h_icon: *mut std::ffi::c_void) -> Option<egui::ColorImage> {
    unsafe {
        let mut icon_info = ICONINFO {
            f_icon: 0,
            x_hotspot: 0,
            y_hotspot: 0,
            hbm_mask: std::ptr::null_mut(),
            hbm_color: std::ptr::null_mut(),
        };

        if GetIconInfo(h_icon, &mut icon_info) == 0 {
            DestroyIcon(h_icon);
            return None;
        }

        let target_bitmap = if !icon_info.hbm_color.is_null() {
            icon_info.hbm_color
        } else {
            icon_info.hbm_mask
        };

        let mut bm = BITMAP {
            bm_type: 0,
            bm_width: 0,
            bm_height: 0,
            bm_width_bytes: 0,
            bm_planes: 0,
            bm_bits_pixel: 0,
            bm_bits: std::ptr::null_mut(),
        };

        if GetObjectW(
            target_bitmap,
            std::mem::size_of::<BITMAP>() as i32,
            &mut bm as *mut _ as *mut std::ffi::c_void,
        ) == 0
        {
            if !icon_info.hbm_color.is_null() {
                DeleteObject(icon_info.hbm_color);
            }
            if !icon_info.hbm_mask.is_null() {
                DeleteObject(icon_info.hbm_mask);
            }
            DestroyIcon(h_icon);
            return None;
        }

        let orig_w = bm.bm_width.max(1);
        let orig_h = if icon_info.hbm_color.is_null() {
            (bm.bm_height / 2).max(1)
        } else {
            bm.bm_height.max(1)
        };

        let hdc = GetDC(std::ptr::null_mut());
        if hdc.is_null() {
            if !icon_info.hbm_color.is_null() {
                DeleteObject(icon_info.hbm_color);
            }
            if !icon_info.hbm_mask.is_null() {
                DeleteObject(icon_info.hbm_mask);
            }
            DestroyIcon(h_icon);
            return None;
        }

        let mut pixels = vec![0u8; (orig_w * orig_h * 4) as usize];
        let mut bmi = BITMAPINFO {
            bmi_header: BITMAPINFOHEADER {
                bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                bi_width: orig_w,
                bi_height: -orig_h,
                bi_planes: 1,
                bi_bit_count: 32,
                bi_compression: 0,
                bi_size_image: 0,
                bi_x_pels_per_meter: 0,
                bi_y_pels_per_meter: 0,
                bi_clr_used: 0,
                bi_clr_important: 0,
            },
            bmi_colors: [0; 1],
        };

        let lines = GetDIBits(
            hdc,
            target_bitmap,
            0,
            orig_h as u32,
            pixels.as_mut_ptr() as *mut std::ffi::c_void,
            &mut bmi,
            0,
        );

        ReleaseDC(std::ptr::null_mut(), hdc);
        if !icon_info.hbm_color.is_null() {
            DeleteObject(icon_info.hbm_color);
        }
        if !icon_info.hbm_mask.is_null() {
            DeleteObject(icon_info.hbm_mask);
        }
        DestroyIcon(h_icon);

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
        let mut shfi = SHFILEINFOW {
            h_icon: std::ptr::null_mut(),
            i_icon: 0,
            dw_attributes: 0,
            sz_display_name: [0; 260],
            sz_type_name: [0; 80],
        };

        let himl = SHGetFileInfoW(
            wide_path.as_ptr(),
            0,
            &mut shfi,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX | SHGFI_LARGEICON,
        ) as *mut std::ffi::c_void;

        if !himl.is_null() {
            let h_icon = ImageList_GetIcon(himl, shfi.i_icon, ILD_NORMAL);
            if !h_icon.is_null()
                && let Some(img) = hicon_to_color_image(h_icon)
            {
                return Some(img);
            }
        }

        // 4. フォールバック: ショートカットそのまま（矢印付き）で確実に取得
        let mut shfi_direct = SHFILEINFOW {
            h_icon: std::ptr::null_mut(),
            i_icon: 0,
            dw_attributes: 0,
            sz_display_name: [0; 260],
            sz_type_name: [0; 80],
        };

        let res = SHGetFileInfoW(
            wide_path.as_ptr(),
            0,
            &mut shfi_direct,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );

        if res != 0 && !shfi_direct.h_icon.is_null() {
            return hicon_to_color_image(shfi_direct.h_icon);
        }
    }

    None
}
