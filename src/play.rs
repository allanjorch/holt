//! Linux playback. The world is rendered on this thread and written straight
//! to ALSA's default device, which is PipeWire on this machine. Another
//! backend would call `World::render` the same way.

#[cfg(not(target_os = "linux"))]
use holt::SAMPLE_RATE;
use std::os::raw::c_void;

pub struct Device {
    pcm: *mut c_void,
}

impl Device {
    pub fn open() -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        {
            linux::open()
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = SAMPLE_RATE;
            Err("This build plays through ALSA, which is Linux. --trace prints the forest without sound.".to_string())
        }
    }

    pub fn write(&mut self, interleaved: &[f32]) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            linux::write(self.pcm, interleaved)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (self, interleaved);
            Err("no audio device".to_string())
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        linux::close(self.pcm);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::Device;
    use holt::SAMPLE_RATE;
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_int, c_uint, c_ulong, c_void};

    const PLAYBACK: c_int = 0;
    const RW_INTERLEAVED: c_int = 3;
    const FLOAT_LE: c_int = 14;

    #[link(name = "asound")]
    unsafe extern "C" {
        fn snd_pcm_open(
            pcm: *mut *mut c_void,
            name: *const c_char,
            stream: c_int,
            mode: c_int,
        ) -> c_int;
        fn snd_pcm_set_params(
            pcm: *mut c_void,
            format: c_int,
            access: c_int,
            channels: c_uint,
            rate: c_uint,
            soft_resample: c_int,
            latency: c_uint,
        ) -> c_int;
        fn snd_pcm_writei(pcm: *mut c_void, buffer: *const c_void, size: c_ulong) -> isize;
        fn snd_pcm_recover(pcm: *mut c_void, err: c_int, silent: c_int) -> c_int;
        fn snd_pcm_drain(pcm: *mut c_void) -> c_int;
        fn snd_pcm_close(pcm: *mut c_void) -> c_int;
        fn snd_strerror(err: c_int) -> *const c_char;
    }

    pub fn open() -> Result<Device, String> {
        let name = CString::new("default").expect("device name");
        let mut pcm = std::ptr::null_mut();
        let rc = unsafe { snd_pcm_open(&mut pcm, name.as_ptr(), PLAYBACK, 0) };
        if rc < 0 || pcm.is_null() {
            return Err(open_error(rc));
        }
        // 100 ms lets the forest render between writes without a callback.
        let rc = unsafe {
            snd_pcm_set_params(pcm, FLOAT_LE, RW_INTERLEAVED, 2, SAMPLE_RATE, 1, 100_000)
        };
        if rc < 0 {
            unsafe { snd_pcm_close(pcm) };
            return Err(open_error(rc));
        }
        Ok(Device { pcm })
    }

    pub fn write(pcm: *mut c_void, mut samples: &[f32]) -> Result<(), String> {
        let mut stalled = 0u32;
        while !samples.is_empty() {
            let frames = (samples.len() / 2) as c_ulong;
            let rc = unsafe { snd_pcm_writei(pcm, samples.as_ptr().cast(), frames) };
            if rc < 0 {
                stalled += 1;
                if stalled > 8 {
                    return Err(format!(
                        "The audio device stopped: {}. --trace prints the forest without sound.",
                        strerror(rc as c_int)
                    ));
                }
                let recovered = unsafe { snd_pcm_recover(pcm, rc as c_int, 1) };
                if recovered < 0 {
                    return Err(format!(
                        "The audio device stopped: {}. --trace prints the forest without sound.",
                        strerror(rc as c_int)
                    ));
                }
                continue;
            }
            stalled = 0;
            let wrote = rc as usize;
            if wrote == 0 {
                return Err(
                    "The audio device accepted no samples. --trace prints the forest without sound."
                        .to_string(),
                );
            }
            samples = &samples[wrote * 2..];
        }
        Ok(())
    }

    pub fn close(pcm: *mut c_void) {
        if pcm.is_null() {
            return;
        }
        unsafe {
            snd_pcm_drain(pcm);
            snd_pcm_close(pcm);
        }
    }

    fn open_error(rc: c_int) -> String {
        format!(
            "Could not open the audio device (ALSA default): {}. --trace prints the forest without sound.",
            strerror(rc)
        )
    }

    fn strerror(err: c_int) -> String {
        unsafe {
            let text = snd_strerror(err);
            if text.is_null() {
                format!("error {err}")
            } else {
                CStr::from_ptr(text).to_string_lossy().into_owned()
            }
        }
    }
}
