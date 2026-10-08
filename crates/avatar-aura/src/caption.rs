// Dark caption bar through DWM, the one place the app needs a Win32 call.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Copies out the raw handle so the attribute can be re-applied after the window is shown.
pub fn handle(window: &impl HasWindowHandle) -> Option<RawWindowHandle> {
    window.window_handle().ok().map(|h| h.as_raw())
}

/// Asks DWM for the dark caption; a value set before the first show is dropped on the first resize, so it is applied again once visible.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn darken(handle: RawWindowHandle) -> Result<(), String> {
    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
    let RawWindowHandle::Win32(win32) = handle else {
        return Err("not a Win32 window".into());
    };
    let dark: i32 = 1;
    // SAFETY: the hwnd comes from a live window handle and the value outlives the call.
    let hr = unsafe {
        DwmSetWindowAttribute(
            win32.hwnd.get() as _,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&dark as *const i32).cast(),
            std::mem::size_of::<i32>() as u32,
        )
    };
    if hr < 0 {
        return Err(format!("DwmSetWindowAttribute: {hr:#x}"));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn darken(_handle: RawWindowHandle) -> Result<(), String> {
    Ok(())
}
