/*! Thread-local-scoped variables.

These unfortunately have to exist as a side-effect of being a DLL plugin that
has no way to pass variables in from top. This includes our DLLMain.

Fortunately, Tera-Term is a mostly single-threaded application. AFAICT, plugins
also run in a single thread. So we can use the thread_local macro.
*/

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::path::PathBuf;

use std::ptr;

use super::sfl::{self, Frame, MagicMatcher, SflLoader};
use super::tt;

use windows::Win32::Foundation::*;

pub struct State {
    #[allow(unused)]
    pub ts: tt::PTTSet,
    pub cv: tt::PComVar,
    pub orig_readfile: tt::TReadFile,
    pub activity: Activity,
    pub matcher: MagicMatcher,
    pub sfl_loader: Option<SflLoader<File>>,
    pub last_frame_sent: Option<u32>,
    pub last_frame_acked: Option<u32>,
    pub filename: Option<PathBuf>,
    pub addr: u32,
    pub curr_frame: Option<Box<Frame>>,
}

#[derive(PartialEq, Clone, Copy)]
pub enum Activity {
    Inactive,
    LookForMagic,
    // WritePacket,
    WaitResp,
    WaitFinalResp,
}

thread_local! {
    pub static OUR_HINST: Cell<HINSTANCE> = Cell::new(HINSTANCE(ptr::null_mut()));
    pub static TTX_LITEX_STATE: RefCell<State> = RefCell::new(State {
        ts: ptr::null_mut(),
        cv: ptr::null_mut(),
        orig_readfile: None,
        activity: Activity::Inactive,
        matcher: MagicMatcher::new(sfl::MAGIC),
        sfl_loader: None,
        last_frame_acked: None,
        last_frame_sent: None,
        filename: None,
        addr: 0x40000000,
        curr_frame: None
    });
}
