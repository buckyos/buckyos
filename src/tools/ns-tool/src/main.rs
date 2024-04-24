use std::fs::File;
use std::io::Read;
use bucky_name_service::DnsTxtCodec;
use clap::{Arg, Command, value_parser};
use sfo_result::err as ns_err;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum NsToolErrorCode {
    #[default]
    Failed,
    OpenFileError,
    ReadFileError,
}

pub type NsToolError = sfo_result::Error<NsToolErrorCode>;
pub type NsToolResult<T> = sfo_result::Result<T>;


fn main() {
    let matches = Command::new("ns-tool")
        .about("A tool for name service")
        .version("0.1.0")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(Command::new("encode")
            .about("Encode the contents of a file into a DNS configurable record")
            .arg(Arg::new("file")
                .help("The file to encode")
                .required(true)
                .short('f')
                .long("file"))
            .arg(Arg::new("txt-limit")
                .help("The maximum length of a TXT record")
                .short('l')
                .long("limit")
                .value_parser(value_parser!(usize))
                .default_value("1024"))
        ).get_matches();

    match matches.subcommand() {
        Some(("encode", encode_matches)) => {
            let file: &String = encode_matches.get_one("file").unwrap();
            let txt_limit: usize = *encode_matches.get_one("txt-limit").unwrap();
            match encode_file(file, txt_limit) {
                Ok(list) => {
                    for item in list {
                        println!("{}", item);
                    }
                },
                Err(e) => {
                    println!("{}", e);
                }
            }
        }
        _ => unreachable!(),
    }
}

fn encode_file(file: &String, txt_limit: usize) -> NsToolResult<Vec<String>> {
    let mut file = File::open(file).map_err(|e| {
        ns_err!(NsToolErrorCode::OpenFileError, "Failed to open file: {}", e)
    })?;
    let mut contents = String::new();
    let read_len = file.read_to_string(&mut contents).map_err(|e| {
        ns_err!(NsToolErrorCode::ReadFileError, "Failed to read file: {}", e)
    })?;

    let content = match serde_json::from_str::<serde_json::Value>(&contents[..read_len]) {
        Ok(json) => {
            json.to_string()
        },
        Err(_) => {
            contents
        }
    };
    let list = DnsTxtCodec::encode(content.as_bytes(), txt_limit).map_err(|e| {
        ns_err!(NsToolErrorCode::Failed, "Failed to encode file: {}", e)
    })?;

    Ok(list)
}
