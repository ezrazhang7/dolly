//! Dolly editor shell (milestone 4).
//!
//! Thin Tauri 2 backend: session state + solver commands. All camera/cursor
//! math stays in dolly-core; the frontend is a static no-bundler page in `ui/`
//! that calls these commands and animates the result. Recording uses the real
//! `WinRecorder` on Windows and falls back to `MockRecorder` everywhere so UI
//! development never requires Windows (AGENTS.md milestone 4).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Mutex;

use dolly_core::camera::{self, CameraFrame, ZoomSegment};
use dolly_core::cursor::{self, CursorConfig, CursorFrame};
use dolly_core::events::RawEvent;
use dolly_core::project::{CameraSettings, CursorSettings, Project, StyleSettings};
use dolly_capture::{MockRecorder, Recorder, RecordingArtifacts};
use serde::Serialize;
use tauri::State;

/// The open document: immutable raw artifacts + the editable project data.
struct Session {
    events: Vec<RawEvent>,
    project: Project,
    /// Absolute path of the capture (None for mock sessions — no video).
    video_path: Option<PathBuf>,
    /// Recording folder (capture.mp4 + events.jsonl + project.json) — this is
    /// the `.dolly` project folder. None for mock sessions.
    dir: Option<PathBuf>,
}

#[derive(Default)]
struct AppState {
    session: Mutex<Option<Session>>,
    #[cfg(windows)]
    recorder: Mutex<Option<dolly_capture::win::WinRecorder>>,
}

#[derive(Serialize)]
struct SessionDto {
    project: Project,
    video_path: Option<String>,
    /// MouseDown moments (t, x, y) — drives click ripples and gives manual
    /// zoom blocks a sensible default center.
    clicks: Vec<(f64, f64, f64)>,
    mock: bool,
}

#[derive(Serialize)]
struct PlanDto {
    fps: f64,
    frame_times_ms: Vec<f64>,
    camera: Vec<CameraFrame>,
    cursor: Vec<CursorFrame>,
}

fn build_session(artifacts: RecordingArtifacts, video_path: Option<PathBuf>, dir: Option<PathBuf>) -> Session {
    let mut project = Project::new(artifacts.meta);
    project.segments = Some(camera::generate_segments(&artifacts.events, &project.camera.config));
    Session { events: artifacts.events, project, video_path, dir }
}

fn session_dto(s: &Session) -> SessionDto {
    let clicks = s
        .events
        .iter()
        .filter_map(|e| match *e {
            RawEvent::MouseDown { t, x, y, .. } => Some((t, x, y)),
            _ => None,
        })
        .collect();
    SessionDto {
        project: s.project.clone(),
        video_path: s.video_path.as_ref().map(|p| p.display().to_string()),
        clicks,
        mock: s.video_path.is_none(),
    }
}

#[tauri::command]
fn load_mock(state: State<AppState>) -> Result<SessionDto, String> {
    let mut rec = MockRecorder::default();
    rec.start().map_err(|e| e.to_string())?;
    let artifacts = rec.stop().map_err(|e| e.to_string())?;
    let session = build_session(artifacts, None, None);
    let dto = session_dto(&session);
    *state.session.lock().unwrap() = Some(session);
    Ok(dto)
}

#[tauri::command]
fn start_recording(app: tauri::AppHandle, state: State<AppState>) -> Result<(), String> {
    #[cfg(windows)]
    {
        use tauri::Manager;
        if state.recorder.lock().unwrap().is_some() {
            return Err("already recording".into());
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let dir = app
            .path()
            .video_dir()
            .map_err(|e| e.to_string())?
            .join("Dolly")
            .join(format!("rec-{stamp}.dolly"));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut rec = dolly_capture::win::WinRecorder::new(dir.join("capture.mp4"));
        rec.start().map_err(|e| format!("{e:?}"))?;
        *state.recorder.lock().unwrap() = Some(rec);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (app, state);
        Err("Recording needs the Windows build — use the mock session on this OS.".into())
    }
}

#[tauri::command]
fn stop_recording(state: State<AppState>) -> Result<SessionDto, String> {
    #[cfg(windows)]
    {
        let mut rec = state
            .recorder
            .lock()
            .unwrap()
            .take()
            .ok_or("not recording")?;
        let artifacts = rec.stop().map_err(|e| format!("{e:?}"))?;
        let video_path = PathBuf::from(&artifacts.video_path);
        let dir = video_path.parent().map(|p| p.to_path_buf());
        let session = build_session(artifacts, Some(video_path), dir);
        if let Some(dir) = &session.dir {
            let _ = std::fs::write(dir.join("project.json"), session.project.to_json());
        }
        let dto = session_dto(&session);
        *state.session.lock().unwrap() = Some(session);
        Ok(dto)
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        Err("Recording needs the Windows build.".into())
    }
}

/// Re-solve camera + cursor for the (possibly edited) segments and settings,
/// persisting the edits into the in-memory project (non-destructive: raw
/// events are never touched).
#[tauri::command]
fn solve(
    state: State<AppState>,
    segments: Vec<ZoomSegment>,
    camera: CameraSettings,
    cursor: CursorSettings,
) -> Result<PlanDto, String> {
    let mut guard = state.session.lock().unwrap();
    let s = guard.as_mut().ok_or("no session loaded")?;
    s.project.segments = Some(segments.clone());
    s.project.camera = camera;
    s.project.cursor = cursor;

    let meta = &s.project.meta;
    let (fps, w, h) = (meta.fps, meta.width as f64, meta.height as f64);
    let n_frames = ((meta.duration_ms / 1000.0) * fps).ceil() as usize + 1;
    let frame_times_ms: Vec<f64> = (0..n_frames).map(|i| i as f64 * 1000.0 / fps).collect();

    let cursor_path =
        cursor::solve_cursor_path(&s.events, &frame_times_ms, CursorConfig { omega: cursor.omega });
    let active: &[ZoomSegment] = if camera.enabled { &segments } else { &[] };
    let camera_path = camera::solve_camera(active, &frame_times_ms, w, h, &camera.config);

    Ok(PlanDto { fps, frame_times_ms, camera: camera_path, cursor: cursor_path })
}

/// Re-cluster zoom segments from the raw input log with the given camera
/// config, discarding manual edits (the "Re-detect zooms" button).
#[tauri::command]
fn regenerate_segments(
    state: State<AppState>,
    camera: CameraSettings,
) -> Result<Vec<ZoomSegment>, String> {
    let mut guard = state.session.lock().unwrap();
    let s = guard.as_mut().ok_or("no session loaded")?;
    s.project.camera = camera;
    let segs = camera::generate_segments(&s.events, &s.project.camera.config);
    s.project.segments = Some(segs.clone());
    Ok(segs)
}

/// Persist style edits (pure CSS on the frontend — no re-solve needed).
#[tauri::command]
fn set_style(state: State<AppState>, style: StyleSettings) -> Result<(), String> {
    let mut guard = state.session.lock().unwrap();
    let s = guard.as_mut().ok_or("no session loaded")?;
    s.project.style = style;
    Ok(())
}

/// Write project.json into the recording folder. Mock sessions have no folder.
#[tauri::command]
fn save_project(state: State<AppState>) -> Result<String, String> {
    let guard = state.session.lock().unwrap();
    let s = guard.as_ref().ok_or("no session loaded")?;
    let dir = s
        .dir
        .as_ref()
        .ok_or("mock session has no folder on disk — record on Windows to save")?;
    let path = dir.join("project.json");
    std::fs::write(&path, s.project.to_json()).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command]
fn export_video() -> Result<String, String> {
    Err("Export needs the offline renderer (milestone 3) — not built yet. \
         Your edits are saved non-destructively in project.json."
        .into())
}

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            load_mock,
            start_recording,
            stop_recording,
            solve,
            regenerate_segments,
            set_style,
            save_project,
            export_video,
        ])
        .run(tauri::generate_context!())
        .expect("error while running dolly");
}
