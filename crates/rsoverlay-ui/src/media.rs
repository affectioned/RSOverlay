//! System media control — titles of whatever is playing — via Windows
//! `GlobalSystemMediaTransportControlsSession` (SMTC). Non-Windows stubs out.

#[derive(Clone, Debug, Default)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub source: String,
    pub playing: bool,
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

#[cfg(not(windows))]
pub fn poll() -> Option<MediaInfo> {
    None
}
