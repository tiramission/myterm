//! Search over the terminal, including scrollback.
//!
//! The find bar used to scan the rows the renderer had already copied, which
//! meant it could only see the screen. The library keeps the scrollback, so it
//! keeps the search too: this type drives that search, hands back the matches
//! that land in the viewport for drawing, and walks between them the way
//! `Enter` and `Shift+Enter` expect.
//!
//! The search borrows the terminal rather than copying it, which is why the
//! terminal handle is kept beside the search handle here: the wrapper promises
//! the library that both are alive together, and frees the search first.

use std::ffi::c_void;
use std::ptr;

use crate::sys;
use crate::{Result, Terminal, check};

/// How many feed/tick rounds one pump may run before giving up for this frame.
///
/// A search over a large scrollback is deliberately incremental; the UI asks for
/// a little progress per frame rather than stalling the frame until it is done.
const PUMP_LIMIT: usize = 64;

/// One match, in viewport coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    /// Row in the viewport, zero at the top of the visible area.
    pub row: usize,
    /// First cell of the match on that row.
    pub start: usize,
    /// Last cell of the match on that row.
    pub end: usize,
}

/// A search bound to a terminal.
pub struct Search {
    raw: sys::GhosttySearch,
    /// The terminal the search is bound to, kept so the two can be freed in the
    /// right order and so matches can be converted into viewport coordinates.
    term: sys::GhosttyTerminal,
}

impl Search {
    /// Creates an idle search bound to `terminal`.
    pub fn new(terminal: &Terminal) -> Result<Self> {
        let mut raw: sys::GhosttySearch = ptr::null_mut();
        check(unsafe { sys::ghostty_search_new(ptr::null(), &mut raw, terminal.raw()) })?;
        Ok(Self {
            raw,
            term: terminal.raw(),
        })
    }

    /// Sets the text to look for. An empty needle clears the search.
    ///
    /// Matching is byte-exact except ASCII letters, which compare
    /// case-insensitively — the behaviour a find bar is expected to have.
    pub fn set_needle(&self, needle: &str) {
        let string = sys::GhosttyString {
            ptr: needle.as_ptr(),
            len: needle.len(),
        };
        let _ = unsafe {
            sys::ghostty_search_set(
                self.raw,
                sys::GHOSTTY_SEARCH_OPT_NEEDLE,
                &string as *const sys::GhosttyString as *const c_void,
            )
        };
    }

    /// Makes a bounded amount of progress, feeding the terminal when asked.
    ///
    /// Returns true while there is more work to do, so the caller can spread a
    /// long search over several frames instead of blocking one.
    pub fn pump(&self) -> bool {
        for _ in 0..PUMP_LIMIT {
            let mut status = sys::GHOSTTY_SEARCH_STATUS_COMPLETE;
            if unsafe { sys::ghostty_search_tick(self.raw, &mut status) } != sys::GHOSTTY_SUCCESS {
                return false;
            }
            match status {
                sys::GHOSTTY_SEARCH_STATUS_RUNNING => continue,
                sys::GHOSTTY_SEARCH_STATUS_FEED_REQUIRED => {
                    if unsafe { sys::ghostty_search_feed(self.raw) } != sys::GHOSTTY_SUCCESS {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        true
    }

    /// How many matches there are, and which one is selected.
    pub fn counts(&self) -> (usize, Option<usize>) {
        let mut total: usize = 0;
        let code = unsafe {
            sys::ghostty_search_get(
                self.raw,
                sys::GHOSTTY_SEARCH_DATA_TOTAL_MATCHES,
                &mut total as *mut usize as *mut c_void,
            )
        };
        if code != sys::GHOSTTY_SUCCESS {
            return (0, None);
        }

        let mut selected: usize = 0;
        let code = unsafe {
            sys::ghostty_search_get(
                self.raw,
                sys::GHOSTTY_SEARCH_DATA_SELECTED_INDEX,
                &mut selected as *mut usize as *mut c_void,
            )
        };
        (total, (code == sys::GHOSTTY_SUCCESS).then_some(selected))
    }

    /// Selects the next match (`forward`) or the previous one.
    ///
    /// The library scrolls the viewport so the selection is visible, which is
    /// what makes a match that lives in the scrollback reachable at all.
    pub fn step(&self, forward: bool) -> bool {
        let code = unsafe {
            if forward {
                sys::ghostty_search_select_next(self.raw)
            } else {
                sys::ghostty_search_select_prev(self.raw)
            }
        };
        code == sys::GHOSTTY_SUCCESS
    }

    /// The match the search currently points at, if it is on screen.
    pub fn selected(&self) -> Option<(usize, usize, usize)> {
        let mut selection = sys::GhosttySelection {
            start: sys::GhosttyGridRef {
                node: ptr::null_mut(),
                x: 0,
                y: 0,
            },
            end: sys::GhosttyGridRef {
                node: ptr::null_mut(),
                x: 0,
                y: 0,
            },
            rectangle: false,
        };
        let code = unsafe {
            sys::ghostty_search_get(
                self.raw,
                sys::GHOSTTY_SEARCH_DATA_SELECTED_MATCH,
                &mut selection as *mut sys::GhosttySelection as *mut c_void,
            )
        };
        if code != sys::GHOSTTY_SUCCESS {
            return None;
        }
        let (start_x, start_y) = self.viewport_point(&selection.start)?;
        let (end_x, _) = self.viewport_point(&selection.end)?;
        Some((start_y, start_x, end_x))
    }

    /// The matches on screen, as viewport cells, for drawing highlights.
    ///
    /// Matches the search found on a page that is only partly visible are
    /// converted here and dropped when the conversion says they are off screen.
    pub fn viewport_matches(&self) -> Vec<Match> {
        let mut buffer = sys::GhosttySelectionBuffer {
            ptr: ptr::null_mut(),
            cap: 0,
            len: 0,
        };
        let code = unsafe {
            sys::ghostty_search_get(
                self.raw,
                sys::GHOSTTY_SEARCH_DATA_VIEWPORT_MATCHES,
                &mut buffer as *mut sys::GhosttySelectionBuffer as *mut c_void,
            )
        };
        if code != sys::GHOSTTY_OUT_OF_SPACE || buffer.len == 0 {
            return Vec::new();
        }

        let mut selections: Vec<sys::GhosttySelection> = Vec::with_capacity(buffer.len);
        let mut buffer = sys::GhosttySelectionBuffer {
            ptr: selections.as_mut_ptr(),
            cap: buffer.len,
            len: 0,
        };
        let code = unsafe {
            sys::ghostty_search_get(
                self.raw,
                sys::GHOSTTY_SEARCH_DATA_VIEWPORT_MATCHES,
                &mut buffer as *mut sys::GhosttySelectionBuffer as *mut c_void,
            )
        };
        if code != sys::GHOSTTY_SUCCESS {
            return Vec::new();
        }
        unsafe { selections.set_len(buffer.len) };

        let mut matches = Vec::with_capacity(selections.len());
        for selection in selections {
            let Some(start) = self.viewport_point(&selection.start) else {
                continue;
            };
            let Some(end) = self.viewport_point(&selection.end) else {
                continue;
            };
            matches.push(Match {
                row: start.1,
                start: start.0,
                end: end.0,
            });
        }
        matches
    }

    /// Converts one end of a match into a viewport cell, or `None` when it is
    /// not on screen.
    fn viewport_point(&self, grid_ref: &sys::GhosttyGridRef) -> Option<(usize, usize)> {
        let mut point = sys::GhosttyPointCoordinate { x: 0, y: 0 };
        let code = unsafe {
            sys::ghostty_terminal_point_from_grid_ref(
                self.term,
                grid_ref,
                sys::GHOSTTY_POINT_TAG_VIEWPORT,
                &mut point,
            )
        };
        (code == sys::GHOSTTY_SUCCESS).then_some((point.x as usize, point.y as usize))
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        // The search releases tracked state inside the terminal, so it has to go
        // first; the field order in `Session` guarantees the terminal outlives it.
        unsafe { sys::ghostty_search_free(self.raw) };
    }
}

impl std::fmt::Debug for Search {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Search").finish_non_exhaustive()
    }
}
