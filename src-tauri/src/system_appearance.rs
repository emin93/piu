#[cfg(target_os = "macos")]
fn read_macos_appearance() -> &'static str {
    use objc2_foundation::{NSUserDefaults, ns_string};

    let interface_style =
        NSUserDefaults::standardUserDefaults().stringForKey(ns_string!("AppleInterfaceStyle"));
    if interface_style
        .as_deref()
        .is_some_and(|style| style.to_string().eq_ignore_ascii_case("dark"))
    {
        "dark"
    } else {
        "light"
    }
}

#[cfg(not(target_os = "macos"))]
fn read_macos_appearance() -> &'static str {
    "light"
}

#[tauri::command]
pub fn system_appearance() -> String {
    read_macos_appearance().to_owned()
}
