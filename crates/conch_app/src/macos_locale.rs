use std::ffi::{CStr, CString};
use std::{env, str};

use libc::{LC_ALL, LC_CTYPE, setlocale};
use log::debug;
use objc2::sel;
use objc2_foundation::{NSLocale, NSObjectProtocol};

const FALLBACK_LOCALE: &str = "UTF-8";

/// Set locale environment variables when launched from Finder (which provides
/// a minimal launchd environment without LANG/LC_ALL).
///
/// This mirrors Alacritty's approach: query the system locale via NSLocale and
/// set LC_ALL so that child processes (and the process itself) get proper
/// UTF-8 locale settings.
pub fn set_locale_environment() {
    let env_locale_c = CString::new("").unwrap();
    let env_locale_ptr = unsafe { setlocale(LC_ALL, env_locale_c.as_ptr()) };
    if !env_locale_ptr.is_null() {
        let env_locale = unsafe { CStr::from_ptr(env_locale_ptr).to_string_lossy() };

        // "C" is the default — treat it as unset.
        if env_locale != "C" {
            debug!("Using environment locale: {}", env_locale);
            return;
        }
    }

    let system_locale = system_locale();

    let system_locale_c = CString::new(system_locale.clone()).expect("nul byte in system locale");
    let lc_all = unsafe { setlocale(LC_ALL, system_locale_c.as_ptr()) };

    if lc_all.is_null() {
        debug!("Using fallback locale: {}", FALLBACK_LOCALE);

        let fallback_locale_c = CString::new(FALLBACK_LOCALE).unwrap();
        unsafe { setlocale(LC_CTYPE, fallback_locale_c.as_ptr()) };
        unsafe { env::set_var("LC_CTYPE", FALLBACK_LOCALE) };
    } else {
        debug!("Using system locale: {}", system_locale);
        unsafe { env::set_var("LC_ALL", system_locale) };
    }
}

/// Determine system locale based on language and country code via NSLocale.
fn system_locale() -> String {
    let locale = NSLocale::currentLocale();

    let is_language_code_supported: bool = locale.respondsToSelector(sel!(languageCode));
    let is_country_code_supported: bool = locale.respondsToSelector(sel!(countryCode));
    if is_language_code_supported && is_country_code_supported {
        let language_code = locale.languageCode();
        #[allow(deprecated)]
        if let Some(country_code) = locale.countryCode() {
            format!("{}_{}.UTF-8", language_code, country_code)
        } else {
            "en_US.UTF-8".into()
        }
    } else {
        locale.localeIdentifier().to_string() + ".UTF-8"
    }
}
