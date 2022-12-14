use std::{
    env,
    io::{self, Error, ErrorKind},
    path::Path,
};

use aws_manager::{self, ec2};
use clap::{crate_version, value_parser, Arg, Command};
use tokio::time::{sleep, Duration};

pub const NAME: &str = "aws-ip-provisioner";

pub fn new() -> Command {
    Command::new(NAME)
        .version(crate_version!())
        .about("Provisions the Elastic IP to the local EC2 instance")
        .long_about(
            "


The EC2 instance is automatically fetched.

Commands may run multiple times with idempotency.

Requires IAM instance role of: ec2:AllocateAddress, ec2:AssociateAddress, and ec2:DescribeAddresses.

e.g.,

$ aws-ip-provisioner \
--log-level=info \
--initial-wait-random-seconds=70 \
--id-tag-key=Id \
--id-tag-value=TEST-ID \
--kind-tag-key=Kind \
--kind-tag-value=aws-ip-provisioner \
--mounted-eip-file-path=/data/eip.yaml

",
        )
        .arg(
            Arg::new("LOG_LEVEL")
                .long("log-level")
                .short('l')
                .help("Sets the log level")
                .required(false)
                .num_args(1)
                .value_parser(["debug", "info"])
                .default_value("info"),
        )
        .arg(
            Arg::new("INITIAL_WAIT_RANDOM_SECONDS")
                .long("initial-wait-random-seconds")
                .help("Sets the maximum number of seconds to wait (value chosen at random with the range)")
                .required(false)
                .num_args(1)
                .value_parser(value_parser!(u32))
                .default_value("5"),
        )
        .arg(
            Arg::new("ID_TAG_KEY")
                .long("id-tag-key")
                .help("Sets the key for the EC2 instance 'Id' tag (must be set via EC2 tags, or used for EIP creation)")
                .required(true)
                .num_args(1)
                .default_value("Id"),
        )
        .arg(
            Arg::new("ID_TAG_VALUE")
                .long("id-tag-value")
                .help("Sets the value for the EC2 instance 'Id' tag key (must be set via EC2 tags)")
                .required(true)
                .num_args(1),
        )
        .arg(
            Arg::new("KIND_TAG_KEY")
                .long("kind-tag-key")
                .help("Sets the key for the EC2 instance 'Kind' tag (must be set via EC2 tags, or used for EIP creation)")
                .required(true)
                .num_args(1)
                .default_value("Kind"),
        )
        .arg(
            Arg::new("KIND_TAG_VALUE")
                .long("kind-tag-value")
                .help("Sets the value for the EC2 instance 'Kind' tag key (must be set via EC2 tags)")
                .required(true)
                .num_args(1),
        )
        .arg(
            Arg::new("MOUNTED_EIP_FILE_PATH")
                .long("mounted-eip-file-path")
                .help("Sets the file path to store Elastic IP information mapped to this volume path")
                .required(true)
                .num_args(1)
                .default_value("/data/eip.yaml"),
        )
}

/// Defines flag options.
pub struct Flags {
    pub log_level: String,
    pub initial_wait_random_seconds: u32,

    pub id_tag_key: String,
    pub id_tag_value: String,
    pub kind_tag_key: String,
    pub kind_tag_value: String,

    pub mounted_eip_file_path: String,
}

pub async fn execute(opts: Flags) -> io::Result<()> {
    println!("{} version: {}", NAME, crate_version!());

    // ref. <https://github.com/env-logger-rs/env_logger/issues/47>
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, opts.log_level),
    );
    log::info!("starting 'aws-ip-provisioner'");

    let shared_config = aws_manager::load_config(None).await?;
    let ec2_manager = ec2::Manager::new(&shared_config);

    let ec2_instance_id = ec2::metadata::fetch_instance_id().await.map_err(|e| {
        Error::new(
            ErrorKind::Other,
            format!("failed fetch_instance_id '{}'", e),
        )
    })?;

    let sleep_sec = if opts.initial_wait_random_seconds > 0 {
        random_manager::u32() % opts.initial_wait_random_seconds
    } else {
        0
    };
    if sleep_sec > 0 {
        log::info!("waiting for random seconds {}", sleep_sec);
        sleep(Duration::from_secs(sleep_sec as u64)).await;
    } else {
        log::info!("skipping random sleep...");
    }

    log::info!(
        "checking if the local instance {} has an already created elastic Ip (for reuse) via {}",
        ec2_instance_id,
        opts.mounted_eip_file_path
    );
    let eip = if Path::new(&opts.mounted_eip_file_path).exists() {
        log::info!(
            "mounted EIP file path exists -- loading existing {}",
            opts.mounted_eip_file_path
        );
        ec2::Eip::load(&opts.mounted_eip_file_path)
            .map_err(|e| Error::new(ErrorKind::Other, format!("failed ec2::Eip::load '{}'", e)))?
    } else {
        log::info!("mounted EIP file does not exist in the mounted volume path -- creating one!");
        ec2_manager
            .allocate_eip(
                &opts.id_tag_key,
                &opts.id_tag_value,
                &opts.kind_tag_key,
                &opts.kind_tag_value,
            )
            .await
            .map_err(|e| {
                Error::new(
                    ErrorKind::Other,
                    format!(
                        "failed ec2_manager.allocate_eip {} (retryable {})",
                        e.message(),
                        e.is_retryable()
                    ),
                )
            })?
    };
    eip.sync(&opts.mounted_eip_file_path)?;

    log::info!(
        "checking the instance has already been associated with elastic IP {:?}",
        eip
    );
    let eips = ec2_manager
        .describe_eips_by_instance_id(&ec2_instance_id)
        .await
        .map_err(|e| {
            Error::new(
                ErrorKind::Other,
                format!(
                    "failed ec2_manager.describe_eips_by_instance_id {} (retryable {})",
                    e.message(),
                    e.is_retryable()
                ),
            )
        })?;
    let need_associate_eip = if eips.is_empty() {
        log::info!(
            "no existing EIP found, now associating {:?} to {ec2_instance_id}",
            eip
        );
        true
    } else {
        log::info!("existing EIPs found {:?}", eips);
        let mut found = false;
        for ev in eips.iter() {
            log::info!("address {:?}", ev);
            let allocation_id = ev.allocation_id.to_owned().unwrap();
            if allocation_id == eip.allocation_id {
                log::info!("{ec2_instance_id} already has EIP allocation ID {allocation_id} -- no need to associate once more");
                found = true;
                break;
            }
        }
        !found // if already associated EIP not found, need associate existing one
    };
    if need_associate_eip {
        let _association_id = ec2_manager
            .associate_eip(&eip.allocation_id, &ec2_instance_id)
            .await
            .map_err(|e| {
                Error::new(
                    ErrorKind::Other,
                    format!(
                        "failed ec2_manager.associate_eip {} (retryable {})",
                        e.message(),
                        e.is_retryable()
                    ),
                )
            })?;
    }

    log::info!("successfully provisioned and associated EIP!");
    Ok(())
}
