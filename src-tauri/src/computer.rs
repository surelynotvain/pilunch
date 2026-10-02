//! Computer use: the agent sees the screen and controls the mouse and keyboard.
//!
//! Linux (X11) uses `xdotool` for input and the first available of ImageMagick `import`,
//! `scrot`, `gnome-screenshot` or `spectacle` for screenshots; Wayland uses `grim` and
//! `ydotool`/`wtype`. Windows uses PowerShell (System.Drawing + user32). Every action asks
//! for approval unless the user allows computer use for the chat. Coordinates are in the
//! (downscaled) screenshot's pixels and mapped back to the real screen.

use crate::media;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct Action {
    pub action: String,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub button: Option<String>,
    pub text: Option<String>,
    pub keys: Option<String>,
    pub direction: Option<String>,
    pub amount: Option<u32>,
    pub seconds: Option<f64>,
}

/// Scale of the last screenshot (screen pixels per image pixel).
static SCALE: Mutex<f64> = Mutex::new(1.0);

pub fn tool_definition() -> Value {
    json!({
        "name": "computer",
        "description": "See and control the user's desktop. Actions: screenshot, click {x, y, button?: left|right|middle}, \
double_click {x, y}, move {x, y}, type {text}, key {keys: e.g. \"ctrl+s\", \"Return\", \"alt+Tab\"}, scroll {x, y, \
direction: up|down, amount?}, wait {seconds}. Coordinates are pixels in the latest screenshot. Every action except wait \
returns a fresh screenshot. Take a screenshot first, act in small steps and check the result. Never enter passwords or \
payment details, and stop to ask the user before anything irreversible.",
        "input_schema": {
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["screenshot", "click", "double_click", "move", "type", "key", "scroll", "wait"] },
                "x": { "type": "number" },
                "y": { "type": "number" },
                "button": { "type": "string", "enum": ["left", "right", "middle"] },
                "text": { "type": "string" },
                "keys": { "type": "string" },
                "direction": { "type": "string", "enum": ["up", "down"] },
                "amount": { "type": "integer", "description": "Scroll clicks (default 5)." },
                "seconds": { "type": "number" }
            },
            "required": ["action"]
        }
    })
}

pub fn describe(a: &Action) -> String {
    let at = || match (a.x, a.y) {
        (Some(x), Some(y)) => format!(" at ({x:.0}, {y:.0})"),
        _ => String::new(),
    };
    match a.action.as_str() {
        "screenshot" => "Take a screenshot".into(),
        "click" => format!("{} click{}", a.button.as_deref().unwrap_or("left"), at()),
        "double_click" => format!("Double-click{}", at()),
        "move" => format!("Move the mouse{}", at()),
        "type" => format!("Type “{}”", crate::util::truncate_end(a.text.as_deref().unwrap_or(""), 80)),
        "key" => format!("Press {}", a.keys.as_deref().unwrap_or("")),
        "scroll" => format!("Scroll {}{}", a.direction.as_deref().unwrap_or("down"), at()),
        "wait" => format!("Wait {}s", a.seconds.unwrap_or(1.0)),
        other => other.to_string(),
    }
}

async fn run(program: &str, args: &[String]) -> Result<Vec<u8>, String> {
    let out = crate::process::command(program).args(args).output().await.map_err(|e| format!("{program}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{program} failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

fn have(program: &str) -> bool {
    let path = crate::process::login_path().or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
    std::env::split_paths(&path).any(|d| d.join(program).is_file() || d.join(format!("{program}.exe")).is_file())
}

fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Capture the whole screen as PNG/JPEG bytes.
async fn capture() -> Result<Vec<u8>, String> {
    if cfg!(windows) {
        let script = "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; $b=[System.Windows.Forms.SystemInformation]::VirtualScreen; \
$bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height; $g=[System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.Left,$b.Top,0,0,$bmp.Size); \
$ms=New-Object System.IO.MemoryStream; $bmp.Save($ms,[System.Drawing.Imaging.ImageFormat]::Png); [Convert]::ToBase64String($ms.ToArray())";
        let out = run("powershell.exe", &["-NoProfile".into(), "-Command".into(), script.into()]).await?;
        return media::b64_decode(&String::from_utf8_lossy(&out));
    }
    let tmp = std::env::temp_dir().join(format!("pilunch-shot-{}.png", std::process::id()));
    let t = tmp.display().to_string();
    let candidates: Vec<(&str, Vec<String>, bool)> = if wayland() {
        vec![("grim", vec!["-".into()], false), ("gnome-screenshot", vec!["-f".into(), t.clone()], true), ("spectacle", vec!["-b".into(), "-n".into(), "-o".into(), t.clone()], true)]
    } else {
        vec![
            ("import", vec!["-window".into(), "root".into(), "png:-".into()], false),
            ("scrot", vec!["-o".into(), t.clone()], true),
            ("gnome-screenshot", vec!["-f".into(), t.clone()], true),
            ("spectacle", vec!["-b".into(), "-n".into(), "-o".into(), t.clone()], true),
        ]
    };
    let mut last = String::from("no screenshot tool found: install ImageMagick (import), scrot or grim");
    for (prog, args, to_file) in candidates {
        if !have(prog) {
            continue;
        }
        match run(prog, &args).await {
            Ok(stdout) if !to_file => return Ok(stdout),
            Ok(_) => {
                let bytes = std::fs::read(&tmp).map_err(|e| e.to_string());
                let _ = std::fs::remove_file(&tmp);
                return bytes;
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

pub async fn screenshot() -> Result<media::Shot, String> {
    let bytes = capture().await?;
    let shot = tokio::task::spawn_blocking(move || media::compress(&bytes)).await.map_err(|e| e.to_string())??;
    *SCALE.lock().unwrap() = shot.scale;
    Ok(shot)
}

fn to_screen(a: &Action) -> Result<(i64, i64), String> {
    let (Some(x), Some(y)) = (a.x, a.y) else { return Err(format!("{} needs x and y", a.action)) };
    let s = *SCALE.lock().unwrap();
    Ok(((x * s).round() as i64, (y * s).round() as i64))
}

fn button_num(b: Option<&str>) -> &'static str {
    match b {
        Some("right") => "3",
        Some("middle") => "2",
        _ => "1",
    }
}

/// Perform an input action (not screenshot/wait).
async fn input(a: &Action) -> Result<(), String> {
    if cfg!(windows) {
        return input_windows(a).await;
    }
    let xdo = !wayland() && have("xdotool");
    let s = |v: &str| v.to_string();
    match a.action.as_str() {
        "click" | "double_click" | "move" => {
            let (x, y) = to_screen(a)?;
            if xdo {
                let mut args = vec![s("mousemove"), x.to_string(), y.to_string()];
                if a.action != "move" {
                    args.extend([s("click"), s("--repeat"), s(if a.action == "double_click" { "2" } else { "1" }), s(button_num(a.button.as_deref()))]);
                }
                run("xdotool", &args).await.map(|_| ())
            } else if have("ydotool") {
                run("ydotool", &[s("mousemove"), s("--absolute"), s("-x"), x.to_string(), s("-y"), y.to_string()]).await?;
                if a.action != "move" {
                    let code = match a.button.as_deref() {
                        Some("right") => "0xC1",
                        Some("middle") => "0xC2",
                        _ => "0xC0",
                    };
                    let mut args = vec![s("click"), s(code)];
                    if a.action == "double_click" {
                        args.extend([s("--repeat"), s("2")]);
                    }
                    run("ydotool", &args).await?;
                }
                Ok(())
            } else {
                Err("install xdotool (X11) or ydotool (Wayland) for mouse control".into())
            }
        }
        "type" => {
            let text = a.text.clone().unwrap_or_default();
            if xdo {
                run("xdotool", &[s("type"), s("--delay"), s("12"), s("--"), text]).await.map(|_| ())
            } else if have("wtype") {
                run("wtype", &[s("--"), text]).await.map(|_| ())
            } else if have("ydotool") {
                run("ydotool", &[s("type"), s("--"), text]).await.map(|_| ())
            } else {
                Err("install xdotool (X11) or wtype/ydotool (Wayland) for typing".into())
            }
        }
        "key" => {
            let keys = a.keys.clone().unwrap_or_default();
            if xdo {
                run("xdotool", &[s("key"), s("--"), keys]).await.map(|_| ())
            } else if have("wtype") {
                // wtype: modifiers with -M/-m, keys with -k
                let parts: Vec<&str> = keys.split('+').collect();
                let (mods, key) = parts.split_at(parts.len().saturating_sub(1));
                let mut args = Vec::new();
                for m in mods {
                    args.extend([s("-M"), m.to_lowercase()]);
                }
                args.extend([s("-k"), key.first().copied().unwrap_or("Return").to_string()]);
                for m in mods {
                    args.extend([s("-m"), m.to_lowercase()]);
                }
                run("wtype", &args).await.map(|_| ())
            } else {
                Err("install xdotool (X11) or wtype (Wayland) for key presses".into())
            }
        }
        "scroll" => {
            if a.x.is_some() {
                let mut m = a.clone();
                m.action = "move".into();
                Box::pin(input(&m)).await?;
            }
            let n = a.amount.unwrap_or(5).clamp(1, 50).to_string();
            let btn = if a.direction.as_deref() == Some("up") { "4" } else { "5" };
            if xdo {
                run("xdotool", &[s("click"), s("--repeat"), n, s(btn)]).await.map(|_| ())
            } else if have("ydotool") {
                let dy = if btn == "4" { "-5" } else { "5" };
                run("ydotool", &[s("mousemove"), s("-w"), s("-x"), s("0"), s("-y"), s(dy)]).await.map(|_| ())
            } else {
                Err("install xdotool (X11) or ydotool (Wayland) for scrolling".into())
            }
        }
        other => Err(format!("unknown computer action \"{other}\"")),
    }
}

async fn input_windows(a: &Action) -> Result<(), String> {
    const PRELUDE: &str = "Add-Type -AssemblyName System.Windows.Forms; Add-Type -Name U -Namespace W -MemberDefinition '[DllImport(\"user32.dll\")] public static extern bool SetCursorPos(int x,int y); [DllImport(\"user32.dll\")] public static extern void mouse_event(int f,int x,int y,int d,int e);';";
    let script = match a.action.as_str() {
        "click" | "double_click" | "move" => {
            let (x, y) = to_screen(a)?;
            let (down, up) = match a.button.as_deref() {
                Some("right") => (0x08, 0x10),
                Some("middle") => (0x20, 0x40),
                _ => (0x02, 0x04),
            };
            let times = match a.action.as_str() {
                "move" => 0,
                "double_click" => 2,
                _ => 1,
            };
            format!("{PRELUDE} [W.U]::SetCursorPos({x},{y}); for($i=0;$i -lt {times};$i++){{[W.U]::mouse_event({down},0,0,0,0);[W.U]::mouse_event({up},0,0,0,0)}}")
        }
        "type" => {
            let t = a.text.clone().unwrap_or_default();
            let escaped: String = t.chars().map(|c| if "+^%~(){}[]".contains(c) { format!("{{{c}}}") } else { c.to_string() }).collect();
            format!("{PRELUDE} [System.Windows.Forms.SendKeys]::SendWait('{}')", escaped.replace('\'', "''"))
        }
        "key" => {
            let keys = a.keys.clone().unwrap_or_default();
            let mut seq = String::new();
            let parts: Vec<&str> = keys.split('+').collect();
            for p in &parts[..parts.len().saturating_sub(1)] {
                seq.push_str(match p.to_lowercase().as_str() {
                    "ctrl" | "control" => "^",
                    "alt" => "%",
                    "shift" => "+",
                    _ => "",
                });
            }
            let k = parts.last().copied().unwrap_or_default();
            let named = match k.to_lowercase().as_str() {
                "return" | "enter" => "{ENTER}".to_string(),
                "tab" => "{TAB}".into(),
                "escape" | "esc" => "{ESC}".into(),
                "backspace" => "{BACKSPACE}".into(),
                "delete" => "{DELETE}".into(),
                "up" => "{UP}".into(),
                "down" => "{DOWN}".into(),
                "left" => "{LEFT}".into(),
                "right" => "{RIGHT}".into(),
                "home" => "{HOME}".into(),
                "end" => "{END}".into(),
                f if f.starts_with('f') && f[1..].parse::<u8>().is_ok() => format!("{{{}}}", f.to_uppercase()),
                _ => k.to_lowercase(),
            };
            seq.push_str(&named);
            format!("{PRELUDE} [System.Windows.Forms.SendKeys]::SendWait('{}')", seq.replace('\'', "''"))
        }
        "scroll" => {
            let n = i64::from(a.amount.unwrap_or(5).clamp(1, 50)) * 120;
            let d = if a.direction.as_deref() == Some("up") { n } else { -n };
            let mv = match to_screen(a) {
                Ok((x, y)) => format!("[W.U]::SetCursorPos({x},{y});"),
                Err(_) => String::new(),
            };
            format!("{PRELUDE} {mv} [W.U]::mouse_event(0x0800,0,0,{d},0)")
        }
        other => return Err(format!("unknown computer action \"{other}\"")),
    };
    run("powershell.exe", &["-NoProfile".into(), "-Command".into(), script]).await.map(|_| ())
}

/// Run an action; every action except `wait` ends with a fresh screenshot.
pub async fn perform(a: &Action) -> Result<(String, Option<media::Shot>), String> {
    match a.action.as_str() {
        "screenshot" => {}
        "wait" => {
            let secs = a.seconds.unwrap_or(1.0).clamp(0.1, 30.0);
            tokio::time::sleep(Duration::from_secs_f64(secs)).await;
            return Ok((format!("Waited {secs}s."), None));
        }
        _ => {
            input(a).await?;
            tokio::time::sleep(Duration::from_millis(600)).await;
        }
    }
    let shot = screenshot().await?;
    let text = if a.action == "screenshot" { format!("Screenshot ({}×{}).", shot.width, shot.height) } else { format!("Done: {}. Screenshot ({}×{}).", describe(a), shot.width, shot.height) };
    Ok((text, Some(shot)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_scale_back_to_the_screen() {
        *SCALE.lock().unwrap() = 2.0;
        let a = Action { action: "click".into(), x: Some(100.0), y: Some(50.4), ..Default::default() };
        assert_eq!(to_screen(&a).unwrap(), (200, 101));
        assert!(to_screen(&Action { action: "click".into(), ..Default::default() }).is_err());
        *SCALE.lock().unwrap() = 1.0;
        assert_eq!(describe(&Action { action: "key".into(), keys: Some("ctrl+s".into()), ..Default::default() }), "Press ctrl+s");
        assert_eq!(button_num(Some("right")), "3");
    }
}
