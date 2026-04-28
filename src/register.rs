use anyhow::Result;

pub fn register() -> Result<()> {
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_string_lossy();

    #[cfg(target_os = "windows")]
    {
        register_windows(&exe_str)?;
    }
    #[cfg(target_os = "linux")]
    {
        register_linux(&exe_str)?;
    }
    #[cfg(target_os = "macos")]
    {
        println!("macOS: add tinylogviewer to your PATH, then:");
        println!("  duti -s com.tinylogviewer public.plain-text viewer");
        println!("  (or right-click a .log file → Get Info → Open With → Change All)");
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        println!("Register .log files manually. Binary: {exe_str}");
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn register_windows(exe_str: &str) -> Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcr = RegKey::predef(HKEY_CLASSES_ROOT);

    // Associate .log extension
    let (ext_key, _) = hkcr.create_subkey(".log")?;
    ext_key.set_value("", &"tinylogviewer.logfile")?;

    // Create file type
    let (ftype, _) = hkcr.create_subkey("tinylogviewer.logfile")?;
    ftype.set_value("", &"Log File")?;

    let (cmd, _) = hkcr.create_subkey("tinylogviewer.logfile\\shell\\open\\command")?;
    cmd.set_value("", &format!("\"{}\" \"%1\"", exe_str))?;

    println!("Registered .log → tinylogviewer at HKCR\\.log");
    println!("You may need to run as Administrator for system-wide registration.");
    Ok(())
}

#[cfg(target_os = "linux")]
fn register_linux(exe_str: &str) -> Result<()> {
    use std::fs;
    use std::process::Command;

    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let apps_dir = format!("{home}/.local/share/applications");
    fs::create_dir_all(&apps_dir)?;

    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName=tinylogviewer\nExec={exe_str} %f\nMimeType=text/x-log;text/plain;\nNoDisplay=false\nTerminal=true\n"
    );
    let desktop_path = format!("{apps_dir}/tinylogviewer.desktop");
    fs::write(&desktop_path, desktop)?;

    Command::new("xdg-mime")
        .args(["default", "tinylogviewer.desktop", "text/x-log"])
        .status()
        .ok();

    println!("Created {desktop_path}");
    println!("Associated text/x-log with tinylogviewer.");
    Ok(())
}
