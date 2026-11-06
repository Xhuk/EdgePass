//! Whether the left mouse button is held down right now.
//!
//! Why the widget asks: on Windows a drag is a *native* move loop. The OS
//! repositions the window from the offset it captured when the drag began, so a
//! geometry change applied mid-drag is either swallowed or leaves the pill
//! displaced from the cursor — and the "is it near the edge?" decision taken at
//! that instant is taken on geometry the user is not looking at. A pause in a
//! drag is not a drop, and the button state is the only thing that tells the two
//! apart. See `settleAfterDrag` in `src/App.tsx`.

/// `Some(down)` where the platform can answer, `None` where it cannot — and
/// "cannot answer" must behave exactly like the widget did before this existed
/// (decide right away), never like "held", which would freeze snapping.
#[cfg(windows)]
pub fn left_button_down() -> Option<bool> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    // The high bit is "currently down". A zero return can also mean "nothing
    // happened in this thread's queue since the last call", which is why the
    // bit is what we read and not the whole value.
    let state = unsafe { GetAsyncKeyState(VK_LBUTTON as i32) };
    Some(state as u16 & 0x8000 != 0)
}

#[cfg(not(windows))]
pub fn left_button_down() -> Option<bool> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_without_panicking_while_nobody_is_clicking() {
        // Run by cargo, so no user is holding the button over this process.
        match left_button_down() {
            Some(held) => assert!(!held, "nadie está pulsando el ratón"),
            None => {} // plataforma sin respuesta: es un caso válido
        }
    }
}
