//! Fuzz target: login-script matching on arbitrary session output must never panic
//! (`fogu_term::login_script`); the 4 KiB window bound is a unit test there.

#![no_main]

use fogu_core::LoginScript;
use fogu_term::login_script::LoginScriptMatcher;
use libfuzzer_sys::fuzz_target;

fn script(expect: &str, send: &str, is_regex: bool, optional: bool) -> LoginScript {
    LoginScript {
        expect: expect.to_owned(),
        send: send.to_owned(),
        is_regex,
        optional,
    }
}

fuzz_target!(|data: &[u8]| {
    let scripts = [
        script("", r"\x41\e", false, false),
        script("login:", "me", false, true),
        script(r"[$#] $", "ls", true, false),
        script("pass", r"é\n", false, false),
    ];
    let (mut matcher, _) = LoginScriptMatcher::new(&scripts, b"\r");
    // Fed across calls: escape sequences and matches split between reads.
    for chunk in data.chunks(7) {
        let _ = matcher.process(chunk);
    }
});
