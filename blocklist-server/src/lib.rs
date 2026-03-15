pub mod db;

pub const CLASSIFICATIONS_HTML: &str =
  include_str!("../templates/classifications.html");
pub const CLASSIFICATIONS_CSS: &[u8] =
  include_bytes!("../static/classifications.css");
pub const ELM_JS: &[u8] = include_bytes!("../static/elm.js");
