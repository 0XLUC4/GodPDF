//! Explorer thumbnails of PDF files: a COM thumbnail provider that renders page 1.
//!
//! The app embeds this DLL, writes it to its data folder and registers it under its PDF
//! file type (`services/association.rs`), so the thumbnails show while GodPDF is the
//! default PDF app. Explorer draws the GodPDF icon in the corner (`TypeOverlay`).
#![cfg(windows)]

use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_POINTER, E_UNEXPECTED, S_FALSE,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl, IStream};
use windows::Win32::UI::Shell::PropertiesSystem::{
    IInitializeWithStream, IInitializeWithStream_Impl,
};
use windows::Win32::UI::Shell::{
    IThumbnailProvider, IThumbnailProvider_Impl, WTS_ALPHATYPE, WTSAT_RGB,
};
use windows::core::{BOOL, GUID, HRESULT, IUnknown, Interface, Ref, Result, implement};

/// Must match `THUMBS_CLSID` in `crates/app/src/services/association.rs`.
const CLSID: GUID = GUID::from_u128(0x033b742f_20ff_421e_821f_d1b9c902aa8a);

/// Files bigger than this get no thumbnail (the whole file is read into memory).
const MAX_FILE_BYTES: usize = 512 * 1024 * 1024;

#[implement(IThumbnailProvider, IInitializeWithStream)]
#[derive(Default)]
struct Provider {
    bytes: Mutex<Option<Arc<Vec<u8>>>>,
}

impl IInitializeWithStream_Impl for Provider_Impl {
    fn Initialize(&self, stream: Ref<IStream>, _mode: u32) -> Result<()> {
        let stream = stream.ok()?;
        let bytes = read_all(stream)?;
        *self.bytes.lock().map_err(|_| E_UNEXPECTED)? = Some(Arc::new(bytes));
        Ok(())
    }
}

impl IThumbnailProvider_Impl for Provider_Impl {
    fn GetThumbnail(&self, cx: u32, bitmap: *mut HBITMAP, alpha: *mut WTS_ALPHATYPE) -> Result<()> {
        if bitmap.is_null() || alpha.is_null() {
            return Err(E_POINTER.into());
        }
        let bytes = self
            .bytes
            .lock()
            .map_err(|_| E_UNEXPECTED)?
            .clone()
            .ok_or(E_UNEXPECTED)?;
        // A broken PDF must not take the host process down.
        let page = std::panic::catch_unwind(|| {
            godpdf_render::Document::open(bytes, None)?.thumbnail(0, cx.max(1))
        })
        .map_err(|_| E_FAIL)?
        .map_err(|_| E_FAIL)?;
        unsafe {
            *bitmap = to_hbitmap(&page)?;
            *alpha = WTSAT_RGB;
        }
        Ok(())
    }
}

/// The whole stream, read from where it stands (the start, as Explorer hands it).
fn read_all(stream: &IStream) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = vec![0u8; 1 << 16];
    loop {
        let mut read = 0u32;
        let status = unsafe {
            stream.Read(
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                Some(&mut read),
            )
        };
        status.ok()?;
        if read == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..read as usize]);
        if bytes.len() > MAX_FILE_BYTES {
            return Err(E_FAIL.into());
        }
    }
}

/// A top-down 32-bit DIB of the opaque page (RGBA to BGRA).
fn to_hbitmap(page: &godpdf_render::Bitmap) -> Result<HBITMAP> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: page.width as i32,
            biHeight: -(page.height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels: *mut c_void = std::ptr::null_mut();
    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut pixels, None, 0)? };
    let target = unsafe { std::slice::from_raw_parts_mut(pixels.cast::<u8>(), page.rgba.len()) };
    for (to, from) in target.chunks_exact_mut(4).zip(page.rgba.chunks_exact(4)) {
        to.copy_from_slice(&[from[2], from[1], from[0], 255]);
    }
    Ok(bitmap)
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let provider: IUnknown = Provider::default().into();
        unsafe { provider.query(iid, object).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

/// COM entry point: the class factory of [`CLSID`].
#[unsafe(no_mangle)]
extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    if clsid.is_null() || object.is_null() {
        return E_POINTER;
    }
    unsafe { *object = std::ptr::null_mut() };
    if unsafe { *clsid } != CLSID {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = Factory.into();
    unsafe { factory.query(iid, object) }
}

/// Never unloaded while the host lives: simplest, and the host is short-lived anyway.
#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Document, Object, Stream, dictionary};
    use windows::Win32::Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW, HGDIOBJ};
    use windows::Win32::UI::Shell::SHCreateMemStream;

    /// One 200 x 100 pt page, black square at the left.
    fn pdf() -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"0 0 0 rg 0 0 100 100 re f".to_vec(),
        ));
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 100.into()],
            "Contents" => content,
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn thumbnail(bytes: &[u8], size: u32) -> Result<(BITMAP, Vec<u8>)> {
        let stream = unsafe { SHCreateMemStream(Some(bytes)) }.ok_or(E_FAIL)?;
        let provider: IInitializeWithStream = Provider::default().into();
        unsafe { provider.Initialize(&stream, 0)? };
        let provider: IThumbnailProvider = provider.cast()?;
        let mut bitmap = HBITMAP::default();
        let mut alpha = WTS_ALPHATYPE::default();
        unsafe { provider.GetThumbnail(size, &mut bitmap, &mut alpha)? };
        assert_eq!(alpha, WTSAT_RGB);
        let mut info = BITMAP::default();
        unsafe {
            GetObjectW(
                HGDIOBJ(bitmap.0),
                size_of::<BITMAP>() as i32,
                Some((&raw mut info).cast()),
            )
        };
        let pixels = unsafe {
            std::slice::from_raw_parts(
                info.bmBits.cast::<u8>(),
                (info.bmWidthBytes * info.bmHeight) as usize,
            )
        }
        .to_vec();
        let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
        Ok((info, pixels))
    }

    #[test]
    fn renders_first_page_at_requested_size() {
        let (info, pixels) = thumbnail(&pdf(), 256).unwrap();
        assert_eq!((info.bmWidth, info.bmHeight), (256, 128));
        // Top-left is the black square, top-right is white paper.
        assert_eq!(&pixels[..4], &[0, 0, 0, 255]);
        let right = (info.bmWidthBytes - 4) as usize;
        assert_eq!(&pixels[right..right + 4], &[255, 255, 255, 255]);
    }

    #[test]
    fn broken_file_fails_without_crashing() {
        assert!(thumbnail(b"not a pdf", 256).is_err());
    }

    #[test]
    fn factory_serves_only_our_class() {
        let mut object = std::ptr::null_mut();
        let other = GUID::from_u128(1);
        assert_eq!(
            DllGetClassObject(&other, &IClassFactory::IID, &mut object),
            CLASS_E_CLASSNOTAVAILABLE
        );
        assert_eq!(
            DllGetClassObject(&CLSID, &IClassFactory::IID, &mut object),
            windows::Win32::Foundation::S_OK
        );
        assert!(!object.is_null());
        drop(unsafe { IClassFactory::from_raw(object) });
    }
}
