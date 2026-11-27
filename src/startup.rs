//! Finder launches have no terminal. Hand the bundled .command file to Terminal;
//! console launches keep their original stdio and controlling terminal.
#[cfg(target_os = "macos")]
pub fn open_terminal_if_needed(args: &[String]) -> anyhow::Result<bool> {
    use std::io::IsTerminal;
    use std::process::Command;
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        return Ok(false);
    }
    let exe = std::env::current_exe()?;
    let Some(contents) = exe.parent().and_then(|p| p.parent()) else {
        return Ok(false);
    };
    if contents.file_name().is_none_or(|n| n != "Contents")
        || !contents.join("Info.plist").is_file()
    {
        return Ok(false);
    }
    anyhow::ensure!(
        args.is_empty(),
        "для запуска с аргументами используйте yamusiq в терминале"
    );
    let status = Command::new("/usr/bin/open")
        .args(["-a", "Terminal"])
        .arg(contents.join("Resources/Launch.command"))
        .status()?;
    anyhow::ensure!(status.success(), "не удалось открыть Terminal для yamusiq");
    Ok(true)
}

#[cfg(not(target_os = "macos"))]
pub fn open_terminal_if_needed(_: &[String]) -> anyhow::Result<bool> {
    Ok(false)
}
