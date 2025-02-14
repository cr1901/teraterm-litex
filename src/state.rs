/*! Statically-scoped variables.

These unfortunately have to exist as a side-effect of being a DLL plugin that
has no way to pass variables in from top. This includes our DLLMain. */

use std::fs::File;
use std::path::PathBuf;
use std::sync::Mutex;

use std::env;
use std::ffi::OsString;

use log::*;
use once_cell::sync::OnceCell;
use stderrlog;

use super::sfl::{self, MagicMatcher, SflLoader};
use super::tt;
use super::Error;

use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

// SAFETY: Tera-Term is a mostly single-threaded application. AFAICT, plugins
// also run in a single thread. This variable is inaccessible outside plugin
// context and is only accessed via Mutex. Therefore it is accessed only
// by a single thread.
unsafe impl Send for State {}

pub struct State {
    pub ts: tt::PTTSet,
    pub cv: tt::PComVar,
    pub orig_readfile: tt::TReadFile,
    pub orig_writefile: tt::TWriteFile,
    pub file_menu: Option<HMENU>,
    pub transfer_menu: Option<HMENU>,
    pub activity: Activity,
    pub matcher: MagicMatcher,
    pub sfl_loader: Option<SflLoader<File>>,
    pub last_frame_sent: Option<u32>,
    pub last_frame_acked: Option<u32>,
    pub filename: Option<PathBuf>,
    pub addr: u32,
}

#[derive(PartialEq)]
pub enum Activity {
    Inactive,
    LookForMagic,
    // WritePacket,
    WaitResp,
    WaitFinalResp
}

// SAFETY: Same rationale as above. I think it's Windows' problem to make sure
// it accesses the HINSTANCE properly. I just provide it when asked :).
unsafe impl Send for OurHInstance {}

struct OurHInstance(HINSTANCE);

static TTX_LITEX_STATE: Mutex<Option<State>> = Mutex::new(None);
static OUR_HINST: Mutex<OnceCell<OurHInstance>> = Mutex::new(OnceCell::new());

pub fn init_state_var(ts: tt::PTTSet, cv: tt::PComVar) -> Result<(), Error> {
    if cfg!(debug_assertions) {
        let _ = stderrlog::new().verbosity(log::Level::Trace).init();
    } else {
        let _ = stderrlog::new().quiet(true).init();
    }

    let mut filename: Option<PathBuf> = None;
    let mut activity: Activity = Activity::Inactive;
    let addr = 0x40000000;
    let mut sfl_loader: Option<SflLoader<File>> = None;

    if cfg!(debug_assertions) {
        if let Ok(f) = env::var("TTX_LITEX_KERNEL") {
            debug!(target: "TTXInit", "Found TTX_LITEX_KERNEL override: {} {:?}", f, env::current_dir());

            let path = PathBuf::from(OsString::from(f));
            match SflLoader::open(path.clone(), addr) {
                Ok(ldr) => {
                    debug!(target: "TTXInit", "Forcing TTXLiteX directly into LookForMagic state");
                    filename = Some(path);
                    activity = Activity::LookForMagic;
                    sfl_loader = Some(ldr);
                }
                Err(e) => {
                    error!(target: "TTXInit", "Could not force TTXLiteX into LookForMagic state: {}", e);
                }
            }
        }
    }

    let mut s = TTX_LITEX_STATE
        .try_lock()
        .map_err(|_| Error::CouldntUnlock("init_state_var"))?;

    *s = Some(State {
        ts,
        cv,
        orig_readfile: None,
        orig_writefile: None,
        file_menu: None,
        transfer_menu: None,
        activity,
        matcher: MagicMatcher::new(sfl::MAGIC),
        sfl_loader,
        last_frame_acked: None,
        last_frame_sent: None,
        filename,
        addr,
    });

    Ok(())
}

pub fn with_state_var<T, F>(f: F) -> Result<T, Error>
where
    F: FnOnce(&mut State) -> Result<T, Error>,
{
    let mut state_guard = TTX_LITEX_STATE
        .try_lock()
        .map_err(|_| Error::CouldntUnlock("TTXState"))?;

    let state = state_guard.as_mut().ok_or(Error::WasEmpty("TTXState"))?;

    f(state)
}

pub fn init_hinst_var(dll_module: HINSTANCE) -> Result<(), Error> {
    let hinst = OUR_HINST
        .try_lock()
        .map_err(|_| Error::CouldntUnlock("init_hinst_var"))?;
    let _ = hinst.set(OurHInstance(dll_module));

    Ok(())
}

pub fn get_hinst_var() -> Result<HINSTANCE, Error> {
    let hinst_guard = OUR_HINST
        .try_lock()
        .map_err(|_| Error::CouldntUnlock("OurHInstance"))?;
    let hinst = hinst_guard.get().ok_or(Error::WasEmpty("OurHInstance"))?;

    Ok(hinst.0)
}
