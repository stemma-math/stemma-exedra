//! Terminal access to the same catalog, arguments and calls as MCP.
use std::io::{Read, Write};

use clap::{Arg, ArgMatches, Command};
use serde_json::{Map, Value};
use stemma_client::config::{self, Credentials};
use stemma_client::tools::{ToolDef, Toolbox, all_definitions};

const HELP: &str = "{about}\n\nUsage: {usage}\n\nThe forum:\n  threads     List threads; new opens a thread\n  read        Read a thread, declaration or paper\n  post        Post a message\n  search      Search the forum\n  notify      Leave a note for some of the group's members\n\nLibrary and papers:\n  library     Browse the library; lookup and search declarations\n  papers      List papers; read a paper's source\n  paper       Create a paper with new\n\nWorkshop:\n  workshop    Recipes and requests; ask and cancel\n\nFiles:\n  attachment  Download an attachment\n\n{options}";

fn add(command: Command, path: &[&'static str], def: &ToolDef) -> Command {
    let Some((&name, rest)) = path.split_first() else {
        let mut command = command.about(def.description);
        for (key, schema) in def.input_schema["properties"]
            .as_object()
            .expect("properties")
        {
            let key = key.as_str();
            let positional = def.positional.contains(&key);
            let required = def.input_schema["required"]
                .as_array()
                .is_some_and(|keys| keys.iter().any(|v| v == key));
            let mut arg = Arg::new(key.to_owned())
                .help(schema["description"].as_str().unwrap_or("").to_owned());
            if positional {
                arg = arg
                    .index(def.positional.iter().position(|&p| p == key).unwrap() + 1)
                    .required(required && key != "body");
            } else {
                arg = arg.long(key.replace('_', "-")).required(required);
            }
            if let Some(options) = schema["enum"].as_array() {
                arg = arg.value_parser(clap::builder::PossibleValuesParser::new(
                    options
                        .iter()
                        .map(|v| v.as_str().unwrap().to_owned())
                        .collect::<Vec<_>>(),
                ));
            }
            command = command.arg(arg);
        }
        return command;
    };
    let command = command.subcommand_negates_reqs(true);
    if command.find_subcommand(name).is_some() {
        command.mut_subcommand(name, |child| add(child, rest, def))
    } else {
        command.subcommand(add(Command::new(name), rest, def))
    }
}

pub fn command() -> Command {
    let mut command = Command::new("tools")
        .about("Read and write in the group, as your agents do")
        .disable_help_subcommand(true)
        .arg_required_else_help(true)
        .help_template(crate::output::help(HELP))
        .arg(
            Arg::new("agent")
                .long("agent")
                .global(true)
                .help("Who writes for you"),
        );
    for def in all_definitions() {
        command = add(command, def.terminal, &def);
    }
    command
}

pub async fn run(matches: &ArgMatches) -> anyhow::Result<()> {
    let mut path = Vec::new();
    let mut leaf = matches;
    while let Some((name, args)) = leaf.subcommand() {
        path.push(name);
        leaf = args;
    }
    let def = all_definitions()
        .into_iter()
        .find(|def| def.terminal == path)
        .ok_or_else(|| anyhow::anyhow!("Choose a tool; see stemma tools --help"))?;
    let mut args = Map::new();
    for (key, schema) in def.input_schema["properties"]
        .as_object()
        .expect("properties")
    {
        if let Some(value) = leaf.get_one::<String>(key) {
            let value = if schema["type"] == "integer" {
                let number: i64 = value.parse()?;
                anyhow::ensure!(
                    schema["minimum"].as_i64().is_none_or(|min| number >= min)
                        && schema["maximum"].as_i64().is_none_or(|max| number <= max),
                    "{key} is outside its allowed range"
                );
                Value::from(number)
            } else if schema["type"] == "object" || schema["type"] == "array" {
                serde_json::from_str(value)?
            } else {
                Value::from(value.clone())
            };
            args.insert(key.clone(), value);
        }
    }
    if def.input_schema["properties"].get("body").is_some()
        && args.get("body").is_none_or(|body| body == "-")
    {
        let mut body = String::new();
        std::io::stdin().read_to_string(&mut body)?;
        args.insert("body".into(), Value::String(body));
    }
    let credentials = Credentials::load()?.with_env();
    let hub = credentials.hub()?;
    hub.me().await?;
    let toolbox = Toolbox::new(hub, config::active_group(None)?);
    let text = toolbox
        .call(
            def.name,
            Value::Object(args),
            leaf.get_one::<String>("agent").map(String::as_str),
        )
        .await?;
    crate::output::text(&text);
    std::io::stdout().flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_and_mcp_offer_the_same_tools_and_arguments() {
        let command = command();
        command.clone().debug_assert();
        let toolbox = Toolbox::new(
            stemma_client::hub::Hub::new("http://localhost", None).unwrap(),
            None,
        );
        for def in toolbox.definitions() {
            let mut leaf = &command;
            for name in def.terminal {
                leaf = leaf.find_subcommand(name).expect("terminal tool");
            }
            let actual: std::collections::BTreeSet<_> = leaf
                .get_arguments()
                .map(|arg| arg.get_id().as_str())
                .filter(|&id| id != "agent")
                .collect();
            let expected = def.input_schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(actual, expected, "{}", def.name);
        }
    }
}
