#![windows_subsystem = "windows"]

mod http;
mod providers;
mod store;
mod util;

use providers::Win;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering::*};
use std::sync::Mutex;
use util::{fmt_dur, now, wide};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_TRAY: u32 = WM_APP + 1;
const WM_DONE: u32 = WM_APP + 2;
const ID_REFRESH: usize = 1;
const ID_COOKIE: usize = 2;
const ID_AUTORUN: usize = 3;
const ID_CONFIG: usize = 4;
const ID_QUIT: usize = 5;
const TIMER_POLL: usize = 1;
const TIMER_RESUME: usize = 2;
const CF_UNICODETEXT: u32 = 13;
const POPUP_W: i32 = 330;

// ---------------------------------------------------------------- shared state

struct Svc {
    name: &'static str,
    plan: String,
    wins: Vec<Win>,
    err: Option<String>,
}

static SVCS: Mutex<Vec<Svc>> = Mutex::new(Vec::new());
static UPDATED: AtomicU64 = AtomicU64::new(0);
static REFRESHING: AtomicBool = AtomicBool::new(false);
static MAIN: AtomicIsize = AtomicIsize::new(0);
static POPUP: AtomicIsize = AtomicIsize::new(0);
static ICON: AtomicIsize = AtomicIsize::new(0);
static FONT: AtomicIsize = AtomicIsize::new(0);
static FONT_B: AtomicIsize = AtomicIsize::new(0);
static LAST_CLOSE: AtomicU64 = AtomicU64::new(0);
static TASKBAR_CREATED: AtomicU64 = AtomicU64::new(0);
static LAST_TIP: Mutex<String> = Mutex::new(String::new());

fn lock() -> std::sync::MutexGuard<'static, Vec<Svc>> {
    SVCS.lock().unwrap_or_else(|e| e.into_inner())
}

fn hwnd(a: &AtomicIsize) -> HWND {
    a.load(Relaxed) as HWND
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

fn level_color(left: f32) -> u32 {
    if left < 15.0 {
        rgb(234, 67, 53)
    } else if left < 40.0 {
        rgb(251, 188, 5)
    } else {
        rgb(52, 168, 83)
    }
}

// -------------------------------------------------------------------- fetching

#[cfg(debug_assertions)]
static DEMO: AtomicBool = AtomicBool::new(false);
static KEEP_OPEN: AtomicBool = AtomicBool::new(false);

#[cfg(debug_assertions)]
fn load_demo() {
    let w = |l: &str, left: f32, h: i64| Win { label: l.into(), left, reset_at: Some(now() + h * 3600) };
    let mut g = lock();
    g.clear();
    g.push(Svc { name: "Claude", plan: "max".into(), wins: vec![w("5h", 58.0, 2), w("7d", 82.0, 90)], err: None });
    g.push(Svc { name: "OpenAI", plan: "plus".into(), wins: vec![w("5h", 12.0, 1), w("7d", 33.0, 120)], err: Some("토큰 만료 - 이 PC에서 codex를 한 번 실행하세요".into()) });
    g.push(Svc { name: "OpenCode Go", plan: String::new(), wins: vec![w("5h", 99.0, 4), w("7d", 85.0, 50), w("30d", 8.0, 600)], err: None });
    UPDATED.store(now() as u64 - 120, SeqCst);
}

fn refresh() {
    #[cfg(debug_assertions)]
    if DEMO.load(SeqCst) {
        return;
    }
    if REFRESHING.swap(true, SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let cfg = store::load();
        let results = [
            ("Claude", providers::claude::fetch(cfg.auto_refresh)),
            ("OpenAI", providers::openai::fetch(&cfg)),
            ("OpenCode Go", providers::opencode::fetch(&cfg.cookie)),
        ];
        {
            let mut g = lock();
            if g.is_empty() {
                for (name, _) in &results {
                    g.push(Svc { name, plan: String::new(), wins: vec![], err: None });
                }
            }
            for (svc, (_, res)) in g.iter_mut().zip(results) {
                match res {
                    Ok((plan, wins)) => {
                        svc.plan = plan;
                        svc.wins = wins;
                        svc.err = None;
                    }
                    Err(e) => svc.err = Some(e), // keep the last good numbers visible
                }
            }
        }
        // full error text for diagnosis: the popup truncates long messages
        let log: String = lock()
            .iter()
            .filter_map(|s| s.err.as_ref().map(|e| format!("{}: {e}\n", s.name)))
            .collect();
        let log_path = store::path().with_file_name("last-error.txt");
        if log.is_empty() {
            let _ = std::fs::remove_file(&log_path);
        } else {
            let _ = std::fs::write(&log_path, log);
        }
        UPDATED.store(now() as u64, SeqCst);
        REFRESHING.store(false, SeqCst);
        unsafe { PostMessageW(hwnd(&MAIN), WM_DONE, 0, 0) };
    });
}

// ------------------------------------------------------------------------ tray

unsafe fn tray_data(h: HWND) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = h;
    nid.uID = 1;
    nid.uCallbackMessage = WM_TRAY;
    nid
}

fn tooltip(svcs: &[Svc]) -> String {
    let mut parts = Vec::new();
    for s in svcs {
        let short = match s.name {
            "OpenCode Go" => "Go",
            n => n,
        };
        if s.wins.is_empty() {
            parts.push(format!("{short} !"));
        } else {
            let w: Vec<String> = s.wins.iter().take(3).map(|w| format!("{} {:.0}%", w.label, w.left)).collect();
            parts.push(format!("{short} {}{}", w.join(" "), if s.err.is_some() { " !" } else { "" }));
        }
    }
    parts.join("\n")
}

/// Static app icon: three usage bars on a dark rounded tile.
unsafe fn make_icon() -> HICON {
    let size = GetSystemMetrics(SM_CXSMICON).max(16);
    let sdc = GetDC(null_mut());
    let dc = CreateCompatibleDC(sdc);
    let mut bi: BITMAPINFO = std::mem::zeroed();
    bi.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: size,
        biHeight: -size,
        biPlanes: 1,
        biBitCount: 32,
        ..std::mem::zeroed()
    };
    let mut bits: *mut std::ffi::c_void = null_mut();
    let color = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    let old = SelectObject(dc, color);

    // all-GDI drawing keeps alpha at 0, so Windows falls back to the (opaque) mask
    fill(dc, RECT { left: 0, top: 0, right: size - 1, bottom: size - 1 }, rgb(32, 33, 36), size / 4);
    let m = (size / 8).max(2); // margin
    let inner_w = size - 2 * m;
    let bar_h = ((size - 2 * m) * 5 / 18).max(2);
    let gap = ((size - 2 * m - 3 * bar_h) / 2).max(1);
    let y0 = m + (size - 2 * m - (3 * bar_h + 2 * gap)) / 2;
    let fills = [(inner_w, rgb(52, 168, 83)), (inner_w * 7 / 10, rgb(251, 188, 5)), (inner_w * 4 / 10, rgb(66, 133, 244))];
    for (i, (w, c)) in fills.iter().enumerate() {
        let top = y0 + i as i32 * (bar_h + gap);
        let track = RECT { left: m, top, right: m + inner_w - 1, bottom: top + bar_h - 1 };
        fill(dc, track, rgb(70, 74, 80), 0);
        fill(dc, RECT { right: m + w - 1, ..track }, *c, 0);
    }

    let mask = CreateBitmap(size, size, 1, 1, null());
    let ii = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
    let icon = CreateIconIndirect(&ii);
    SelectObject(dc, old);
    DeleteObject(color);
    DeleteObject(mask);
    DeleteDC(dc);
    ReleaseDC(null_mut(), sdc);
    icon
}

unsafe fn update_tray(add: bool) {
    let h = hwnd(&MAIN);
    let tip = tooltip(&lock());
    {
        let mut last = LAST_TIP.lock().unwrap_or_else(|e| e.into_inner());
        if !add && *last == tip {
            return; // nothing changed: skip the round trip to explorer
        }
        *last = tip.clone();
    }
    let mut icon = ICON.load(Relaxed) as HICON;
    if icon.is_null() {
        icon = make_icon();
        ICON.store(icon as isize, SeqCst);
    }
    let mut nid = tray_data(h);
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.hIcon = icon;
    for (d, c) in nid.szTip.iter_mut().zip(tip.encode_utf16().take(127)) {
        *d = c;
    }
    Shell_NotifyIconW(if add { NIM_ADD } else { NIM_MODIFY }, &nid);
    trim_memory();
}

fn trim_memory() {
    unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
}

// ----------------------------------------------------------------------- popup

#[derive(Clone, Copy)]
enum Row {
    Head(usize),
    Bar(usize, usize),
    Msg(usize),
    Gap,
    Foot,
}

fn row_h(r: Row) -> i32 {
    match r {
        Row::Head(_) => 26,
        Row::Bar(..) => 22,
        Row::Msg(_) => 20,
        Row::Gap => 8,
        Row::Foot => 22,
    }
}

fn rows(svcs: &[Svc]) -> Vec<Row> {
    let mut v = Vec::new();
    for (i, s) in svcs.iter().enumerate() {
        v.push(Row::Head(i));
        if s.err.is_some() || s.wins.is_empty() {
            v.push(Row::Msg(i));
        }
        for j in 0..s.wins.len() {
            v.push(Row::Bar(i, j));
        }
        v.push(Row::Gap);
    }
    v.push(Row::Foot);
    v
}

fn total_h(svcs: &[Svc]) -> i32 {
    rows(svcs).into_iter().map(row_h).sum::<i32>() + 20
}

fn scale() -> f32 {
    unsafe { GetDpiForSystem() as f32 / 96.0 }
}

unsafe fn fill(dc: HDC, rc: RECT, color: u32, round: i32) {
    let brush = CreateSolidBrush(color);
    let (ob, op) = (SelectObject(dc, brush), SelectObject(dc, GetStockObject(NULL_PEN)));
    if round > 0 {
        RoundRect(dc, rc.left, rc.top, rc.right + 1, rc.bottom + 1, round, round);
    } else {
        Rectangle(dc, rc.left, rc.top, rc.right + 1, rc.bottom + 1);
    }
    SelectObject(dc, ob);
    SelectObject(dc, op);
    DeleteObject(brush);
}

unsafe fn text(dc: HDC, s: &str, mut rc: RECT, color: u32, flags: u32, bold: bool) {
    SelectObject(dc, if bold { hwnd(&FONT_B) } else { hwnd(&FONT) } as HGDIOBJ);
    SetTextColor(dc, color);
    let w: Vec<u16> = s.encode_utf16().collect();
    if w.is_empty() {
        return; // an empty Vec's dangling pointer crashes user32
    }
    DrawTextW(dc, w.as_ptr(), w.len() as i32, &mut rc, flags | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS);
}

unsafe fn paint(dc: HDC, width: i32, height: i32) {
    let s = scale();
    let px = |v: i32| (v as f32 * s).round() as i32;
    let (bg, fg, dim, track) = (rgb(32, 33, 36), rgb(232, 234, 237), rgb(154, 160, 167), rgb(60, 64, 67));
    fill(dc, RECT { left: 0, top: 0, right: width, bottom: height }, rgb(70, 74, 80), 0);
    fill(dc, RECT { left: 1, top: 1, right: width - 1, bottom: height - 1 }, bg, 0);
    SetBkMode(dc, TRANSPARENT as i32);

    let g = lock();
    let pad = px(14);
    let mut y = px(10);
    for r in rows(&g) {
        let h = px(row_h(r));
        let line = RECT { left: pad, top: y, right: width - pad, bottom: y + h };
        match r {
            Row::Head(i) => {
                text(dc, g[i].name, line, fg, DT_LEFT, true);
                text(dc, &g[i].plan, line, dim, DT_RIGHT, false);
            }
            Row::Msg(i) => {
                let (msg, c) = match &g[i].err {
                    Some(e) => (e.as_str(), rgb(242, 153, 74)),
                    None => ("데이터 없음", dim),
                };
                text(dc, msg, line, c, DT_LEFT, false);
            }
            Row::Bar(i, j) => {
                let w = &g[i].wins[j];
                let label_w = px(62);
                let info_w = px(104);
                text(dc, &w.label, RECT { right: line.left + label_w, ..line }, dim, DT_LEFT, false);
                let bar_l = line.left + label_w;
                let bar_r = line.right - info_w;
                let cy = y + h / 2;
                let bh = px(4);
                let track_rc = RECT { left: bar_l, top: cy - bh, right: bar_r, bottom: cy + bh };
                fill(dc, track_rc, track, px(8));
                let fw = ((bar_r - bar_l) as f32 * w.left / 100.0) as i32;
                if fw > px(3) {
                    fill(dc, RECT { right: bar_l + fw, ..track_rc }, level_color(w.left), px(8));
                }
                let reset = w.reset_at.map_or("-".into(), |t| fmt_dur(t - now()));
                text(dc, &format!("{:.0}%  {}", w.left, reset), RECT { left: bar_r + px(8), ..line }, fg, DT_RIGHT, false);
            }
            Row::Gap => {}
            Row::Foot => {
                let u = UPDATED.load(Relaxed) as i64;
                let msg = if REFRESHING.load(Relaxed) {
                    "갱신 중...".to_string()
                } else if u == 0 {
                    "아직 갱신 안 됨".to_string()
                } else {
                    let m = ((now() - u) / 60).max(0);
                    let ago = if m < 1 { "방금".to_string() } else if m < 60 { format!("{m}분 전") } else { format!("{}시간 전", m / 60) };
                    format!("{ago} 갱신 · 남은 % 기준")
                };
                text(dc, &msg, line, dim, DT_LEFT, false);
            }
        }
        y += h;
    }
}

/// Work area (screen minus taskbar) of the monitor containing `pt`.
unsafe fn work_area(pt: POINT) -> RECT {
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST), &mut mi);
    mi.rcWork
}

unsafe fn show_popup() {
    if POPUP.load(Relaxed) != 0 {
        return;
    }
    let s = scale();
    let h = (total_h(&lock()) as f32 * s) as i32;
    let w = (POPUP_W as f32 * s) as i32;
    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    let wa = work_area(pt);
    let x =(pt.x - w / 2).clamp(wa.left + 8, (wa.right - w - 8).max(wa.left + 8));
    let y = if pt.y > (wa.top + wa.bottom) / 2 { wa.bottom - h - 8 } else { wa.top + 8 };
    let hi = GetModuleHandleW(null());
    let p = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST, wide("LLMTrackerPopup").as_ptr(), null(), WS_POPUP,
        x, y, w, h, null_mut(), null_mut(), hi, null(),
    );
    POPUP.store(p as isize, SeqCst);
    ShowWindow(p, SW_SHOW);
    SetForegroundWindow(p);
    if now() as u64 - UPDATED.load(Relaxed) > 60 {
        refresh();
    }
}

unsafe extern "system" fn popup_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            let s = scale();
            let mk = |weight: u32| {
                CreateFontW(
                    -((12.5 * s) as i32), 0, 0, 0, weight as i32, 0, 0, 0, DEFAULT_CHARSET as u32,
                    OUT_DEFAULT_PRECIS as u32, CLIP_DEFAULT_PRECIS as u32, CLEARTYPE_QUALITY as u32,
                    DEFAULT_PITCH as u32, wide("Segoe UI").as_ptr(),
                ) as isize
            };
            FONT.store(mk(FW_NORMAL), SeqCst);
            FONT_B.store(mk(FW_SEMIBOLD), SeqCst);
            0
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(h, &mut ps);
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(h, &mut rc);
            let mem = CreateCompatibleDC(dc);
            let bmp = CreateCompatibleBitmap(dc, rc.right, rc.bottom);
            let old = SelectObject(mem, bmp);
            paint(mem, rc.right, rc.bottom);
            BitBlt(dc, 0, 0, rc.right, rc.bottom, mem, 0, 0, SRCCOPY);
            SelectObject(mem, old);
            DeleteObject(bmp);
            DeleteDC(mem);
            EndPaint(h, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_ACTIVATE if (wp & 0xFFFF) as u32 == WA_INACTIVE && !KEEP_OPEN.load(Relaxed) => {
            DestroyWindow(h);
            0
        }
        WM_KEYDOWN if wp as u32 == 0x1B /* VK_ESCAPE */ => {
            DestroyWindow(h);
            0
        }
        WM_DESTROY => {
            POPUP.store(0, SeqCst);
            LAST_CLOSE.store(GetTickCount64(), SeqCst);
            for f in [&FONT, &FONT_B] {
                DeleteObject(f.swap(0, SeqCst) as HGDIOBJ);
            }
            trim_memory();
            0
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

// ------------------------------------------------------------------ menu & misc

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

unsafe fn autorun_enabled() -> bool {
    RegGetValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide("LLMTracker").as_ptr(), RRF_RT_REG_SZ, null_mut(), null_mut(), null_mut()) == 0
}

unsafe fn set_autorun(on: bool) {
    store::save_autorun(on);
    let mut key: HKEY = null_mut();
    if RegOpenKeyExW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), 0, KEY_SET_VALUE, &mut key) != 0 {
        return;
    }
    let name = wide("LLMTracker");
    if on {
        if let Ok(exe) = std::env::current_exe() {
            let v = wide(&format!("\"{}\"", exe.display()));
            RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, v.as_ptr() as *const u8, (v.len() * 2) as u32);
        }
    } else {
        RegDeleteValueW(key, name.as_ptr());
    }
    RegCloseKey(key);
}

unsafe fn clipboard_text() -> Option<String> {
    if OpenClipboard(hwnd(&MAIN)) == 0 {
        return None;
    }
    let mut out = None;
    let h = GetClipboardData(CF_UNICODETEXT);
    if !h.is_null() {
        let p = GlobalLock(h) as *const u16;
        if !p.is_null() {
            let mut n = 0;
            while *p.add(n) != 0 && n < 8192 {
                n += 1;
            }
            out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
            GlobalUnlock(h);
        }
    }
    CloseClipboard();
    out
}

unsafe fn msgbox(text: &str, flags: u32) {
    MessageBoxW(hwnd(&MAIN), wide(text).as_ptr(), wide("LLM Tracker").as_ptr(), flags | MB_TOPMOST);
}

unsafe fn cookie_from_clipboard() {
    let cookie = clipboard_text().map(|t| providers::opencode::normalize_cookie(&t)).unwrap_or_default();
    if cookie.len() < 16 {
        msgbox(
            "클립보드에서 OpenCode 세션 쿠키를 찾지 못했습니다.\n\n\
             opencode.ai 로그인 상태에서 F12 → Application → Cookies → https://opencode.ai → \
             '__Host-console_session' 의 Value(st_로 시작)를 복사한 뒤 다시 시도하세요.",
            MB_OK | MB_ICONWARNING,
        );
        return;
    }
    match store::save_cookie(&cookie) {
        Ok(()) => {
            refresh();
            msgbox("쿠키를 저장했습니다 (Windows 계정으로 암호화). 사용량을 갱신합니다.", MB_OK | MB_ICONINFORMATION);
        }
        Err(e) => msgbox(&format!("저장 실패: {e}"), MB_OK | MB_ICONERROR),
    }
}

unsafe fn context_menu(h: HWND) {
    let menu = CreatePopupMenu();
    let add = |id: usize, label: &str, checked: bool| {
        AppendMenuW(menu, MF_STRING | if checked { MF_CHECKED } else { 0 }, id, wide(label).as_ptr());
    };
    add(ID_REFRESH, "지금 새로고침", false);
    add(ID_COOKIE, "OpenCode 쿠키 붙여넣기 (클립보드)", false);
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    add(ID_AUTORUN, "Windows 시작 시 실행", autorun_enabled());
    add(ID_CONFIG, "설정 파일 열기", false);
    add(ID_QUIT, "종료", false);
    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    SetForegroundWindow(h);
    TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, h, null());
    PostMessageW(h, WM_NULL, 0, 0);
    DestroyMenu(menu);
}

unsafe extern "system" fn main_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            match lp as u32 {
                WM_LBUTTONUP => {
                    if POPUP.load(Relaxed) != 0 {
                        DestroyWindow(hwnd(&POPUP));
                    } else if GetTickCount64() - LAST_CLOSE.load(Relaxed) > 250 {
                        show_popup();
                    }
                }
                WM_RBUTTONUP => context_menu(h),
                _ => {}
            }
            0
        }
        WM_DONE => {
            update_tray(false);
            let p = hwnd(&POPUP);
            if !p.is_null() {
                let hgt = (total_h(&lock()) as f32 * scale()) as i32;
                let mut rc: RECT = std::mem::zeroed();
                GetWindowRect(p, &mut rc);
                let wa = work_area(POINT { x: rc.left, y: rc.top });
                // the content height changed: keep the window inside the work area
                let y = rc.top.min(wa.bottom - 8 - hgt).max(wa.top + 8);
                SetWindowPos(p, null_mut(), rc.left, y, (POPUP_W as f32 * scale()) as i32, hgt, SWP_NOZORDER | SWP_NOACTIVATE);
                InvalidateRect(p, null(), 0);
            }
            0
        }
        WM_TIMER => {
            if wp == TIMER_RESUME {
                KillTimer(h, TIMER_RESUME);
            }
            refresh();
            0
        }
        WM_POWERBROADCAST if wp == 0x12 /* PBT_APMRESUMEAUTOMATIC */ => {
            SetTimer(h, TIMER_RESUME, 15_000, None); // wait for the network to come back
            1
        }
        WM_COMMAND => {
            match wp & 0xFFFF {
                ID_REFRESH => refresh(),
                ID_COOKIE => cookie_from_clipboard(),
                ID_AUTORUN => set_autorun(!autorun_enabled()),
                ID_CONFIG => {
                    store::ensure_exists();
                    ShellExecuteW(null_mut(), wide("open").as_ptr(), wide("notepad.exe").as_ptr(),
                        wide(&format!("\"{}\"", store::path().display())).as_ptr(), null(), SW_SHOWNORMAL);
                }
                ID_QUIT => {
                    DestroyWindow(h);
                }
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            let nid = tray_data(h);
            Shell_NotifyIconW(NIM_DELETE, &nid);
            PostQuitMessage(0);
            0
        }
        m if m as u64 == TASKBAR_CREATED.load(Relaxed) => {
            update_tray(true); // explorer restarted: re-add the icon
            0
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

fn main() {
    unsafe {
        let mutex = CreateMutexW(null(), 0, wide("Local\\LLMTrackerSingleton").as_ptr());
        if mutex.is_null() || GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        store::ensure_exists();
        TASKBAR_CREATED.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) as u64, SeqCst);

        let hi = GetModuleHandleW(null());
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.hInstance = hi;
        wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        let (main_cls, popup_cls) = (wide("LLMTrackerMain"), wide("LLMTrackerPopup"));
        wc.lpfnWndProc = Some(main_proc);
        wc.lpszClassName = main_cls.as_ptr();
        RegisterClassW(&wc);
        wc.lpfnWndProc = Some(popup_proc);
        wc.lpszClassName = popup_cls.as_ptr();
        RegisterClassW(&wc);

        let h = CreateWindowExW(0, main_cls.as_ptr(), null(), WS_POPUP, 0, 0, 0, 0, null_mut(), null_mut(), hi, null());
        MAIN.store(h as isize, SeqCst);
        #[cfg(debug_assertions)]
        if std::env::args().any(|a| a == "--demo") {
            DEMO.store(true, SeqCst);
            load_demo();
        }
        update_tray(true);
        refresh();
        let cfg = store::load();
        // keep the "start with Windows" choice: record an existing registration, restore a lost one
        match (autorun_enabled(), cfg.autorun) {
            (true, false) => store::save_autorun(true),
            (false, true) => set_autorun(true),
            _ => {}
        }
        if std::env::args().any(|a| a == "--show") {
            KEEP_OPEN.store(true, SeqCst);
            show_popup();
        }
        SetTimer(h, TIMER_POLL, (cfg.interval_min * 60_000) as u32, None);

        let mut m: MSG = std::mem::zeroed();
        while GetMessageW(&mut m, null_mut(), 0, 0) > 0 {
            TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
}
