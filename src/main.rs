use std::collections::HashMap;
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use anyhow::Context;
use regex::Regex;
use reqwest::blocking::Client;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[clap(version, about)]
#[command(disable_help_subcommand = true)]
pub struct Command {
    /// File to upload
    pub file: String,

    /// File request url shared by the source
    pub url: String,

    /// Name of the submitter (Firstname Lastname)
    #[clap(short, long)]
    pub name: Option<String>,

    /// Name of the file in the onedrive folder
    #[clap(short, long)]
    pub filename: Option<String>
}

fn main() {

    let command = Command::parse();

    let path = PathBuf::from(command.file);
    if !path.exists() || path.is_dir() {
        println!("File is a directory or does not exist");
    }

    let file = match File::open(&path) {
        Ok(f) => f,
        Err(e) => { println!("Could not open file: {}", e); return;}
    };

    let names = command.name.map(|n| n.split(' ').map(|e| e.to_owned()).collect::<Vec<String>>()).unwrap_or_else(|| vec!["Marcel".to_owned(), "D'Avis".to_owned()]);
    if names.len() != 2 {
        println!("Please specify the name in the following format: 'Firstname Lastname'");
        return;
    }

    let filename = command.filename.unwrap_or_else(|| path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "untitled".to_owned()));

    match upload(&command.url, &filename, names.get(0).unwrap(), names.get(1).unwrap(), file) {
        Ok(result) => {
            if !result { println!("Upload failed, file was rejected") }
        }
        Err(e) => {
            println!("Upload failed: {}", e.to_string())
        }
    }
}

fn upload(url: &str, filename: &str, given_name: &str, family_name: &str, file: File) -> anyhow::Result<bool>{
    let client = Client::new();

    // Get upload page url (the result will be only be a base64 encoded version of the url, but this may change)
    let response = client.get(url).send()?;
    let params = response.url().query().context("no query params")?;
    let id = Regex::new("s=(\\w+)")?.captures(params).and_then(|c| c.get(1)).context("found no capture")?.as_str();

    // Obtain single use token
    let response = client.post("https://api.badgerp.svc.ms/v1.0/token")
        .json(&HashMap::from([("appid", "FileRequestAnonymousUserSignInOnODB")]))
        .send()?;
    let token = response.json::<HashMap<String, String>>()?.get("token").context("token request did not return anonymous token")?.clone();

    // Check permissions, this requests makes the token valid in the first place (and depends on the prefer header for some reason)
    let permissions = client.get(format!("https://redacted.sharepoint.com/personal/redacted/_api/v2.0/shares/u!{id}/permission"))
        .header("Authorization", format!("badger {}", token)).header("Prefer", "redeemsharinglink")
        .send()?;

    let permissions = permissions.text().unwrap();

    let response = client.post("https://api.badgerp.svc.ms/v1.0/tokenexchange")
        .header("Authorization", format!("badger {}", token))
        .json(&HashMap::from([("givenName", given_name), ("familyName", family_name)]))
        .send()?;

    let token = response.json::<HashMap<String, String>>()?.get("token").context("token request did not return anonymous token")?.clone();

    // Start transaction
    let url = format!("https://redacted.sharepoint.com/_api/v2.1/shares/u!{id}/driveItem:/{filename}:/oneDrive.createUploadSession");
    let start = client.post(url)
        .body(r#"{"item":{"@name.conflictBehavior":"rename"}}"#)
        .header("Authorization", format!("badger {}", token)).header("Content-Type", "application/json")
        .send()?;

    let response = start.text()?;
    // Json parsing does not work, so we have to use a regex
    let url = Regex::new(r#""uploadUrl":"(.+)"}"#)?.captures(&response).and_then(|c| c.get(1)).context("couldn't capture upload url")?.as_str();

    // Upload file
    let metadata = file.metadata()?;
    let end = client.put(url)
        .body(file).header("Content-Range", format!("bytes {}-{}/{}", 0, metadata.size() - 1, metadata.size())).send()?;

    Ok(end.status().is_success())
}