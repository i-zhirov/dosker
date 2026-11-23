//! `dosker search`: filter the static registry catalog.

use dosker_image::default_registry_url;
use dosker_image::remote::{PullError, RegistryClient};

use crate::cli::{EXIT_OK, SearchArgs};
use crate::context::Context;
use crate::error::CliError;

pub fn execute(args: SearchArgs, context: &mut Context) -> Result<u8, CliError> {
    let client = RegistryClient::new(&default_registry_url())
        .map_err(|error| CliError::Pull(PullError::Remote(error)))?;
    let catalog = client
        .catalog()
        .map_err(|error| CliError::Pull(PullError::Remote(error)))?;
    let term = args.term.to_lowercase();
    let images = catalog
        .images
        .iter()
        .filter(|image| {
            let name = format!("{}/{}", image.namespace, image.name).to_lowercase();
            name.contains(&term)
                || image.description.to_lowercase().contains(&term)
                || image
                    .tags
                    .keys()
                    .any(|tag| tag.to_lowercase().contains(&term))
        })
        .collect::<Vec<_>>();

    if args.json {
        let filtered = serde_json::json!({
            "schema": catalog.schema,
            "generated_at": catalog.generated_at,
            "registry": catalog.registry,
            "images": images,
        });
        let text = serde_json::to_string(&filtered)
            .map_err(|error| CliError::Message(error.to_string()))?;
        context.console.line(&text)?;
    } else {
        for image in images {
            let tags = image.tags.keys().cloned().collect::<Vec<_>>().join(",");
            context.console.line(&format!(
                "{}/{}  {}  {}  {}",
                image.namespace,
                image.name,
                image.license_summary.as_str(),
                tags,
                image.description
            ))?;
        }
    }
    Ok(EXIT_OK)
}
