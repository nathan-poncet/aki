//! The board for an assistant: a Model Context Protocol server on standard
//! input and output. One JSON-RPC message per line comes in, a typed call
//! goes to the application, its answer goes back as text.
//!
//! Someone who never opens a terminal talks to the board through their
//! assistant: "what waits on me?", "set that one aside until Monday",
//! "start on the review of web!916".

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::entities::desk::Sort;
use crate::entities::task::Bucket;

/// What the assistant may do beyond reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allowed {
    pub set_aside: bool,
    pub start_agent: bool,
}

/// Until when a task is set aside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aside {
    Days(u32),
    UntilItMoves,
}

/// What a tool call asks of the application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    ListTasks {
        side: Option<Bucket>,
        space: Option<String>,
        filter: Option<String>,
        sort: Sort,
        limit: Option<usize>,
    },
    /// Everything about one task, with the brief of what there is to do.
    Task {
        reference: String,
    },
    Spaces,
    Rules,
    SetAside {
        reference: String,
        until: Aside,
    },
    BringBack {
        reference: String,
    },
    StartAgent {
        reference: String,
    },
}

const INSTRUCTIONS: &str = "aki is the user's board of work across GitLab, GitHub and Jira: \
what waits on them (to do) and what they cannot move further (waiting). Use list_tasks to see \
it, most urgent first; get_task for one task with the brief of what is expected; explain_rules \
for why tasks are listed and how urgency is computed. Never post, approve, merge or push \
anything on the user's behalf unless they ask.";

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
        },
    })
}

const REFERENCE: &str = "The task, as the board names it: web!916, notes#12, PROJ-7, its ticket \
                         key or its URL.";

/// The tools on offer, given what is allowed.
pub fn tools(allowed: Allowed) -> Vec<Value> {
    let reference = json!({ "reference": { "type": "string", "description": REFERENCE } });
    let mut tools = vec![
        tool(
            "list_tasks",
            "The user's board: what waits on them and what waits on others, most urgent first. \
             Every task says why it is listed and how its urgency score is made.",
            json!({
                "side": { "type": "string", "enum": ["todo", "waiting"],
                    "description": "Only one side. Default: both." },
                "space": { "type": "string",
                    "description": "Only what belongs to one of the user's spaces (see list_spaces)." },
                "filter": { "type": "string",
                    "description": "A board query: is:todo|waiting|draft|snoozed source: repo: kind: \
                                    top: role: by: heat:hot|warm|calm ticket: priority: agent:, bare \
                                    words, -negation. Example: \"repo:web kind:review -is:draft\"." },
                "sort": { "type": "string",
                    "enum": ["urgency", "rule", "waiting", "updated", "created"],
                    "description": "Default: urgency." },
                "limit": { "type": "integer", "minimum": 1,
                    "description": "At most this many tasks per side." },
            }),
            &[],
        ),
        tool(
            "get_task",
            "One task in full: its reasons, its urgency explained, its links, and the brief of \
             what is expected, ready to work from.",
            reference.clone(),
            &["reference"],
        ),
        tool(
            "list_spaces",
            "The user's spaces (for example work and personal): what each contains, its panes \
             and their filters.",
            json!({}),
            &[],
        ),
        tool(
            "explain_rules",
            "Every rule that puts a task on the board, on which side and in what order, and how \
             the urgency score is computed.",
            json!({}),
            &[],
        ),
    ];
    if allowed.set_aside {
        tools.push(tool(
            "set_aside",
            "Hide a task from the board for a number of days, or until something happens on it. \
             It comes back by itself.",
            json!({
                "reference": { "type": "string", "description": REFERENCE },
                "days": { "type": "integer", "minimum": 1,
                    "description": "For this many days. Omit to wait until the item moves." },
            }),
            &["reference"],
        ));
        tools.push(tool(
            "bring_back",
            "Put a task that was set aside back on the board.",
            reference.clone(),
            &["reference"],
        ));
    }
    if allowed.start_agent {
        tools.push(tool(
            "start_agent",
            "Open the user's coding agent on a task, already told what there is to do, the way \
             the user configured it (a terminal tab, or a new Claude conversation).",
            reference,
            &["reference"],
        ));
    }
    tools
}

/// A tool call's name and arguments, as a typed call.
fn call(name: &str, arguments: &Value, allowed: Allowed) -> Result<Call, String> {
    let text = |key: &str| arguments[key].as_str().map(str::to_string);
    let reference = || {
        text("reference")
            .filter(|reference| !reference.trim().is_empty())
            .ok_or_else(|| "`reference` is required".to_string())
    };
    let number = |key: &str| match &arguments[key] {
        Value::Null => Ok(None),
        value => value
            .as_u64()
            .filter(|number| *number >= 1)
            .map(Some)
            .ok_or_else(|| format!("`{key}` must be a whole number, 1 or more")),
    };
    match name {
        "list_tasks" => Ok(Call::ListTasks {
            side: match text("side") {
                None => None,
                Some(side) => Some(
                    Bucket::from_name(&side)
                        .ok_or_else(|| format!("unknown side \"{side}\" (todo, waiting)"))?,
                ),
            },
            space: text("space"),
            filter: text("filter"),
            sort: match text("sort") {
                None => Sort::Urgency,
                Some(sort) => Sort::from_name(&sort).ok_or_else(|| {
                    format!(
                        "unknown sort \"{sort}\" ({})",
                        Sort::ALL.map(Sort::name).join(", ")
                    )
                })?,
            },
            limit: number("limit")?.map(|limit| limit as usize),
        }),
        "get_task" => Ok(Call::Task {
            reference: reference()?,
        }),
        "list_spaces" => Ok(Call::Spaces),
        "explain_rules" => Ok(Call::Rules),
        "set_aside" if allowed.set_aside => Ok(Call::SetAside {
            reference: reference()?,
            until: match number("days")? {
                Some(days) => Aside::Days(days.min(3_650) as u32),
                None => Aside::UntilItMoves,
            },
        }),
        "bring_back" if allowed.set_aside => Ok(Call::BringBack {
            reference: reference()?,
        }),
        "start_agent" if allowed.start_agent => Ok(Call::StartAgent {
            reference: reference()?,
        }),
        "set_aside" | "bring_back" | "start_agent" => Err(format!(
            "\"{name}\" is turned off in the [mcp] section of aki's configuration"
        )),
        other => Err(format!("unknown tool \"{other}\"")),
    }
}

/// The application's side: does what a call asks, and says it in text.
pub type Handle<'a> = &'a dyn Fn(Call) -> Result<String, String>;

/// The answer to one message; `None` for what expects none.
pub fn answer(message: &Value, allowed: Allowed, handle: Handle) -> Option<Value> {
    // A notification has no id, and gets no answer.
    let id = message.get("id")?.clone();
    let result = |result: Value| json!({ "jsonrpc": "2.0", "id": id, "result": result });
    let said = |text: String, failed: bool| {
        result(json!({ "content": [{ "type": "text", "text": text }], "isError": failed }))
    };
    Some(match message["method"].as_str() {
        Some("initialize") => result(json!({
            // The client's own version when it names one: what is used here
            // has not changed across them.
            "protocolVersion": message["params"]["protocolVersion"]
                .as_str()
                .unwrap_or("2025-06-18"),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "aki", "version": env!("CARGO_PKG_VERSION") },
            "instructions": INSTRUCTIONS,
        })),
        Some("ping") => result(json!({})),
        Some("tools/list") => result(json!({ "tools": tools(allowed) })),
        Some("tools/call") => {
            let params = &message["params"];
            let name = params["name"].as_str().unwrap_or_default();
            match call(name, &params["arguments"], allowed).and_then(handle) {
                Ok(text) => said(text, false),
                Err(refused) => said(refused, true),
            }
        }
        method => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("unknown method \"{}\"", method.unwrap_or_default()),
            },
        }),
    })
}

/// Answers messages until the input ends.
pub fn serve(
    input: impl BufRead,
    mut output: impl Write,
    allowed: Allowed,
    handle: Handle,
) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => answer(&message, allowed, handle),
            Err(error) => Some(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": format!("not JSON: {error}") },
            })),
        };
        if let Some(reply) = reply {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// What to paste into an assistant's configuration to plug the board in.
pub fn setup(program: &str) -> String {
    let entry = json!({ "mcpServers": { "aki": { "command": program, "args": ["mcp"] } } });
    format!(
        "Claude Desktop: Settings › Developer › Edit Config, then add to claude_desktop_config.json\n\
         and restart the app:\n\n{}\n\n\
         Claude Code:\n\n  claude mcp add aki -- {program} mcp\n\n\
         What the assistant may do is set under [mcp] in aki's configuration.\n",
        serde_json::to_string_pretty(&entry).expect("a JSON value always serializes")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const ALL: Allowed = Allowed {
        set_aside: true,
        start_agent: true,
    };
    const READ_ONLY: Allowed = Allowed {
        set_aside: false,
        start_agent: false,
    };

    fn names(allowed: Allowed) -> Vec<String> {
        tools(allowed)
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect()
    }

    fn ask(name: &str, arguments: Value, allowed: Allowed) -> (Value, Vec<Call>) {
        let seen = RefCell::new(Vec::new());
        let message = json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        });
        let reply = answer(&message, allowed, &|call| {
            seen.borrow_mut().push(call);
            Ok("done".to_string())
        })
        .unwrap();
        (reply, seen.into_inner())
    }

    #[test]
    fn a_client_is_greeted_and_shown_the_tools() {
        let hello = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-03-26" },
        });
        let reply = answer(&hello, ALL, &|_| unreachable!()).unwrap();
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(reply["result"]["serverInfo"]["name"], "aki");
        assert!(reply["result"]["capabilities"]["tools"].is_object());

        // A notification gets no answer.
        let ready = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert_eq!(answer(&ready, ALL, &|_| unreachable!()), None);

        let list = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let reply = answer(&list, ALL, &|_| unreachable!()).unwrap();
        assert_eq!(reply["result"]["tools"].as_array().unwrap().len(), 7);
        assert_eq!(reply["result"]["tools"][0]["inputSchema"]["type"], "object");
    }

    #[test]
    fn what_is_turned_off_is_neither_offered_nor_done() {
        assert_eq!(
            names(READ_ONLY),
            ["list_tasks", "get_task", "list_spaces", "explain_rules"]
        );
        assert!(names(ALL).contains(&"start_agent".to_string()));

        let (reply, calls) = ask("start_agent", json!({ "reference": "web!916" }), READ_ONLY);
        assert_eq!(reply["result"]["isError"], true);
        assert!(
            reply["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("[mcp]")
        );
        assert!(calls.is_empty());
    }

    #[test]
    fn calls_arrive_typed() {
        let (reply, calls) = ask(
            "list_tasks",
            json!({ "side": "todo", "filter": "repo:web", "sort": "waiting", "limit": 5 }),
            ALL,
        );
        assert_eq!(reply["result"]["isError"], false);
        assert_eq!(reply["result"]["content"][0]["text"], "done");
        assert_eq!(
            calls,
            [Call::ListTasks {
                side: Some(Bucket::Todo),
                space: None,
                filter: Some("repo:web".into()),
                sort: Sort::Waiting,
                limit: Some(5),
            }]
        );

        let (_, calls) = ask("set_aside", json!({ "reference": "web!916" }), ALL);
        assert_eq!(
            calls,
            [Call::SetAside {
                reference: "web!916".into(),
                until: Aside::UntilItMoves
            }]
        );
        let (_, calls) = ask(
            "set_aside",
            json!({ "reference": "web!916", "days": 3 }),
            ALL,
        );
        assert!(matches!(
            calls[0],
            Call::SetAside {
                until: Aside::Days(3),
                ..
            }
        ));
    }

    #[test]
    fn what_cannot_be_understood_is_said_not_guessed() {
        for (name, arguments) in [
            ("get_task", json!({})),
            ("list_tasks", json!({ "side": "later" })),
            ("list_tasks", json!({ "limit": 0 })),
            ("merge", json!({})),
        ] {
            let (reply, calls) = ask(name, arguments, ALL);
            assert_eq!(reply["result"]["isError"], true, "{name}");
            assert!(calls.is_empty());
        }

        let stranger = json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/list" });
        let reply = answer(&stranger, ALL, &|_| unreachable!()).unwrap();
        assert_eq!(reply["error"]["code"], -32601);
    }

    #[test]
    fn a_conversation_is_one_message_per_line() {
        let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n\nnot json\n\
                     {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                     {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\
                     \"params\":{\"name\":\"explain_rules\"}}\n";
        let mut output = Vec::new();
        serve(input.as_bytes(), &mut output, ALL, &|call| {
            assert_eq!(call, Call::Rules);
            Err("no rules today".to_string())
        })
        .unwrap();
        let replies: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0]["result"], json!({}));
        assert_eq!(replies[1]["error"]["code"], -32700);
        assert_eq!(replies[2]["result"]["isError"], true);
        assert_eq!(replies[2]["result"]["content"][0]["text"], "no rules today");
    }

    #[test]
    fn the_setup_names_the_program() {
        let setup = setup("/usr/local/bin/aki");
        assert!(setup.contains("\"command\": \"/usr/local/bin/aki\""));
        assert!(setup.contains("claude mcp add aki -- /usr/local/bin/aki mcp"));
    }
}
