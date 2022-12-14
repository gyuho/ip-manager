pub mod command;

use std::io;

pub const APP_NAME: &str = "aws-ip-provisioner";

#[tokio::main]
async fn main() -> io::Result<()> {
    let matches = command::new().get_matches();

    let log_level = matches
        .get_one::<String>("LOG_LEVEL")
        .unwrap_or(&String::from("info"))
        .clone();

    let initial_wait_random_seconds = matches
        .get_one::<u32>("INITIAL_WAIT_RANDOM_SECONDS")
        .unwrap_or(&5)
        .clone();

    let id_tag_key = matches.get_one::<String>("ID_TAG_KEY").unwrap().clone();
    let id_tag_value = matches.get_one::<String>("ID_TAG_VALUE").unwrap().clone();
    let kind_tag_key = matches.get_one::<String>("KIND_TAG_KEY").unwrap().clone();
    let kind_tag_value = matches.get_one::<String>("KIND_TAG_VALUE").unwrap().clone();

    let mounted_eip_file_path = matches
        .get_one::<String>("MOUNTED_EIP_FILE_PATH")
        .unwrap_or(&String::from("/data"))
        .clone();

    let opts = command::Flags {
        log_level,
        initial_wait_random_seconds,
        id_tag_key,
        id_tag_value,
        kind_tag_key,
        kind_tag_value,
        mounted_eip_file_path,
    };
    command::execute(opts).await
}
