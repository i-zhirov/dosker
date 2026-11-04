//! `dosker create`: make a container and its private writable disk.

use chrono::{DateTime, Utc};
use dosker_core::{ContainerId, ContainerName};
use dosker_engine::DisplayMode;
use dosker_image::ImageReference;
use dosker_platform::endpoint::control_endpoint;
use dosker_runtime::ContainerRecord;

use crate::adapter;
use crate::cli::{CreateArgs, EXIT_OK};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(
    args: CreateArgs,
    context: &mut Context,
    now: DateTime<Utc>,
) -> Result<u8, CliError> {
    let engine = match args.engine {
        Some(engine) => engine,
        None => match adapter::default_engine() {
            Some(engine) => engine.to_owned(),
            None => {
                return Err(CliError::Usage(String::from(
                    "no engine selected; pass --engine",
                )));
            }
        },
    };
    let display = DisplayMode::parse(&args.display).ok_or_else(|| {
        CliError::Usage(format!(
            "invalid display mode: {}; use none, window, or vnc",
            args.display
        ))
    })?;

    let record = create_container(
        context,
        &args.image,
        args.name,
        engine,
        display,
        args.vnc_port,
        now,
    )?;
    context.console.line(record.id.as_str())?;

    Ok(EXIT_OK)
}

pub fn create_container(
    context: &mut Context,
    image: &str,
    name: Option<String>,
    engine: String,
    display: DisplayMode,
    vnc_port: Option<u16>,
    now: DateTime<Utc>,
) -> Result<ContainerRecord, CliError> {
    if !adapter::is_known(&engine) {
        return Err(CliError::Usage(format!("unknown engine: {engine}")));
    }

    let reference = ImageReference::parse(image)?;
    let image_record = context.images.load(&reference)?;

    let id = ContainerId::generate();
    let name = match name {
        Some(name) => ContainerName::parse(&name)?,
        None => ContainerName::parse(&format!("dosker-{}", &id.as_str()[..8]))?,
    };

    let root = std::path::absolute(context.paths.root())?;
    let endpoint = control_endpoint(&root, &id);
    let record = ContainerRecord::new(
        id.clone(),
        name,
        reference.to_string(),
        engine,
        endpoint.record(),
        now,
    )
    .with_image(
        image_record.digest.clone(),
        image_record.is_booter(),
        image_record.cmd.clone(),
    )
    .with_display(display, vnc_port);

    context.store.create(&record)?;

    let disk = context.paths.disk_file(&id);
    if let Err(error) = context.images.copy_disk_to(&reference, &disk) {
        let _ = context.store.remove(&record);
        return Err(error.into());
    }

    let record = record.with_disk(disk);
    if let Err(error) = context.store.save(&record) {
        let _ = context.store.remove(&record);
        return Err(error.into());
    }

    Ok(record)
}
