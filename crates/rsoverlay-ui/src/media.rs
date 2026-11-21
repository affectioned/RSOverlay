//! System media control — titles of whatever is playing + play/pause
//! / next / prev writes — via Windows
//! `GlobalSystemMediaTransportControlsSession` (SMTC). Non-Windows
//! stubs out.

#[derive(Clone, Debug, Default)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub source: String,
    pub playing: bool,
}

#[derive(Copy, Clone, Debug)]
pub enum MediaCommand {
    PlayPause,
    Next,
    Previous,
}

#[cfg(windows)]
pub fn poll() -> Option<MediaInfo> {
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Mgr;
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status;

    let mgr = Mgr::RequestAsync().ok()?.get().ok()?;
    let session = mgr.GetCurrentSession().ok()?;
    let props = session.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let playback = session.GetPlaybackInfo().ok()?;
    let status = playback.PlaybackStatus().ok()?;
    let title = props.Title().ok()?.to_string();
    let artist = props.Artist().ok()?.to_string();
    let source = session.SourceAppUserModelId().ok()?.to_string();
    Some(MediaInfo {
        title,
        artist,
        source,
        playing: status == Status::Playing,
    })
}

/// Fire a transport command at whatever media app currently owns the
/// SMTC session (Spotify, browser tab, local player, whatever). The
/// toggle semantics for play/pause match what the media keys on your
/// keyboard already do — if nothing is playing and nothing is paused,
/// the command silently fails rather than guessing.
#[cfg(windows)]
pub fn send(cmd: MediaCommand) -> bool {
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Mgr;
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status;

    let Some(mgr) = Mgr::RequestAsync().ok().and_then(|op| op.get().ok()) else {
        return false;
    };
    let Ok(session) = mgr.GetCurrentSession() else {
        return false;
    };
    let result = match cmd {
        MediaCommand::PlayPause => {
            let status = session
                .GetPlaybackInfo()
                .and_then(|p| p.PlaybackStatus())
                .ok();
            match status {
                Some(Status::Playing) => {
                    session.TryPauseAsync().ok().and_then(|op| op.get().ok())
                }
                _ => session.TryPlayAsync().ok().and_then(|op| op.get().ok()),
            }
        }
        MediaCommand::Next => session
            .TrySkipNextAsync()
            .ok()
            .and_then(|op| op.get().ok()),
        MediaCommand::Previous => session
            .TrySkipPreviousAsync()
            .ok()
            .and_then(|op| op.get().ok()),
    };
    result.unwrap_or(false)
}

#[cfg(not(windows))]
pub fn poll() -> Option<MediaInfo> {
    None
}

#[cfg(not(windows))]
pub fn send(_cmd: MediaCommand) -> bool {
    false
}
