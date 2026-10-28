use super::{Command, CommandHandler, CommandType};

use coap_lite::CoapRequest;
use coap_lite::RequestType as Method;
use coap_lite::{CoapOption, option_value::OptionValueU16};
use coap_lite::{MessageClass, ResponseType};

#[derive(PartialEq, Eq)]
enum State {
    WaitGet,
    WaitPut,
    Successful,
    Error(String),
}

pub struct CoapEdit {
    path: String,
    state: State,
}

impl CoapEdit {
    pub fn parse(_cmd: &Command, args: &str) -> Result<CommandType, String> {
        let Some((_edit, path)) = args.split_once(" ") else {
            return Err("usage: `edit PATH` (e.g. `edit /hello`)\n".to_owned());
        };
        Ok(CommandType::CoAP(Box::new(Self {
            path: path.into(),
            state: State::WaitGet,
        })))
    }
}

impl CommandHandler for CoapEdit {
    fn init(&mut self) -> CoapRequest<String> {
        let mut request = CoapRequest::new();
        request.set_method(Method::Get);
        request.set_path(&self.path);
        request
    }

    fn handle(&mut self, payload: &coap_lite::Packet) -> Option<coap_lite::CoapRequest<String>> {
        match self.state {
            State::WaitGet => {
                if payload.header.code != MessageClass::Response(ResponseType::Content) {
                    // Technically this could be a successful state that is not Content, but we're not
                    // requesting an ETag, not creating a new resource etc.
                    self.state = State::Error(format!(
                        "GETting the data was not successful, response code {}",
                        payload.header.code
                    ));
                    return None;
                }

                let old_content = &payload.payload;
                let Ok(content_format) = payload
                    .get_first_option_as::<OptionValueU16>(CoapOption::ContentFormat)
                    .transpose()
                else {
                    self.state = State::Error(format!("Unprocessable content format option"));
                    return None;
                };
                // FIXME: carry ETag, send If-Match

                let suffix = self
                    .path
                    .rsplit_once("/")
                    .map(|(_left, right)| right)
                    .unwrap_or(&self.path);

                // Following https://ratatui.rs/recipes/apps/spawn-vim/ around the editor invocaion

                use crossterm::ExecutableCommand;
                use ratatui::crossterm::terminal::{
                    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
                };
                use std::io::stdout;
                {
                    crate::tui::PULL_EVENTS.store(false, std::sync::atomic::Ordering::Relaxed);
                    std::thread::sleep(crate::tui::POLL_INTERVAL * 2);
                    disable_raw_mode().unwrap();
                    stdout().execute(LeaveAlternateScreen).unwrap();
                }

                let edited = edit::edit_with_builder(
                    old_content,
                    &edit::Builder::new().suffix("-").suffix(suffix),
                );

                {
                    stdout().execute(EnterAlternateScreen).unwrap();
                    enable_raw_mode().unwrap();
                    crate::tui::PULL_EVENTS.store(true, std::sync::atomic::Ordering::Relaxed);
                }

                let edited = edited
                    .map_err(|e| {
                        self.state = State::Error(format!(
                            "Not uploading changed version: editor did not exit successfully: {e}."
                        ))
                    })
                    .ok()?;

                self.state = State::WaitPut;

                let mut request = CoapRequest::new();
                request.set_method(Method::Put);
                request.set_path(&self.path);
                request.message.payload = edited.as_bytes().into();

                Some(request)
            }
            State::WaitPut => {
                if payload.header.code != MessageClass::Response(ResponseType::Changed) {
                    self.state = State::Successful;
                } else {
                    self.state = State::Error(format!(
                        "PUTting the data was not successful, response code {}",
                        payload.header.code
                    ));
                }

                None
            }
            State::Successful => unreachable!("no request is made in this state"),
            State::Error(_) => unreachable!("no request is made in this state"),
        }
    }

    fn want_display(&self) -> bool {
        self.is_finished()
    }

    fn is_finished(&self) -> bool {
        matches!(self.state, State::Successful | State::Error(_))
    }

    fn display(&self, buffer: &mut String) {
        use std::fmt::Write;
        match &self.state {
            State::Successful => write!(buffer, "Edit was applied successfullly.\n").unwrap(),
            State::Error(e) => write!(buffer, "Editing failed: {e}.\n").unwrap(),
            _ => unreachable!("we didn't ask to be displayed yet"),
        }
    }
}
