use tastyworks::Session;

use std::env;
use std::error::Error;
use std::io::{Write, stdin, stdout};

const SESSION_KEY_ENV_VAR: &str = "TASTYWORKS_SESSION_KEY";

pub async fn session_from_env_or_login(_example_name: &str) -> Result<Session, Box<dyn Error>> {
    if let Ok(session_key) = env::var(SESSION_KEY_ENV_VAR) {
        if !session_key.is_empty() {
            return Ok(Session::from_token(session_key));
        }
    }

    login().await
}

async fn login() -> Result<Session, Box<dyn Error>> {
    let mut login = String::new();
    print!("login: ");
    stdout().flush()?;
    stdin().read_line(&mut login)?;
    let login = login.trim_end().to_string();

    let password = rpassword::prompt_password("password (hidden): ")?;

    let mut otp = String::new();
    print!("2fa (press enter if none): ");
    stdout().flush()?;
    stdin().read_line(&mut otp)?;
    let otp = otp.trim_end().to_string();
    let otp = if otp.is_empty() { None } else { Some(otp) };

    Ok(Session::from_credentials(login, password, otp).await?)
}
