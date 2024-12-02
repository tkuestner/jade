use rodio::cpal::SampleRate;
use rodio::source::{Function, SignalGenerator, Source};
use rodio::{OutputStream, OutputStreamHandle, PlayError, Sink, StreamError};
use thiserror::Error;

pub struct Sound {
    #[allow(dead_code)]
    stream: OutputStream,
    #[allow(dead_code)]
    stream_handle: OutputStreamHandle,
    sink: Sink,
}

impl Sound {
    pub fn new() -> Result<Self, SoundError> {
        let (stream, stream_handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&stream_handle)?;

        Ok(Sound {
            stream,
            stream_handle,
            sink,
        })
    }

    pub fn play(&self) {
        let source = SignalGenerator::new(SampleRate(48000), 220.0, Function::Sawtooth)
            .repeat_infinite()
            .amplify(0.1)
            .fade_in(std::time::Duration::from_millis(100));
        self.sink.append(source);
        self.sink.play();
    }

    pub fn pause(&self) {
        // This abrupt stop may lead to a popping noise. It is unclear if rodio currently supports
        // fading out an infinite source.
        self.sink.clear();
    }
}

#[derive(Debug, Error)]
pub enum SoundError {
    #[error(transparent)]
    Play(#[from] PlayError),

    #[error(transparent)]
    Stream(#[from] StreamError),
}
