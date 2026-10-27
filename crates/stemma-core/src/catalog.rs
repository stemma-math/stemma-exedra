//! Read-only tools shared by answers and the hub's orchestrators.
use serde_json::{Value, json};
const GROUP: &str = "The group's short name; the default group if absent.";
#[derive(Clone)]
pub struct ReadTool {
    pub name: &'static str,
    pub terminal: &'static [&'static str],
    pub positional: &'static [&'static str],
    pub description: &'static str,
    pub input_schema: Value,
}
pub fn read_tools() -> Vec<ReadTool> {
    vec![
        ReadTool {
            name: "list_threads",
            terminal: &["threads"],
            positional: &[],
            description: "List the threads of a group, most recently active first, with their \
                          addresses (t12), titles and number of messages.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "group": {"type": "string", "description": GROUP},
                    "kind": {"type": "string", "enum": ["discussion", "chat", "request"],
                             "description": "Only discussions, or only chats."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200,
                              "description": "How many; 30 if absent."}
                }
            }),
        },
        ReadTool {
            name: "read_thread",
            terminal: &["read"],
            positional: &["address"],
            description: "Read a thread, a message or a range of messages, by address: t12 \
                          (the thread), t12#45 (a message) or t12#45-50 (messages 45 to 50). \
                          Links to the web app work too. Each message comes headed by its \
                          address, its author and its time. Declarations (lean:Name) and the \
                          and papers are read too.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "address": {"type": "string", "description": "t12, t12#45, t12#45-50, lean:Name or p/notes."},
                    "file": {"type": "string", "description": "For a paper, the source file to read."},
                    "group": {"type": "string", "description": GROUP}
                },
                "required": ["address"]
            }),
        },
        ReadTool {
            name: "search",
            terminal: &["search"],
            positional: &["query"],
            description: "Search a group's thread titles and messages, regardless of case and \
                          accents: every word (or prefix), and \"phrases\" in quotes, narrowed by \
                          author:<username>, since:/until: (2026-03-01, 7d, 2w), \
                          in:forum|chats|papers|library|p/<slug>|t<n> and \
                          kind:thread|message|agent|person. Results come with their addresses, \
                          to read them with read_thread.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "The words to find, all of them, and operators."},
                    "group": {"type": "string", "description": GROUP},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100,
                              "description": "How many results; 20 if absent."}
                },
                "required": ["query"]
            }),
        },
        ReadTool {
            name: "library_state",
            terminal: &["library"],
            positional: &[],
            description: "How the group's Lean library stands: its last build, how many \
                          declarations it has, which depend on sorry, and its modules.",
            input_schema: json!({
                "type": "object",
                "properties": {"group": {"type": "string", "description": GROUP}}
            }),
        },
        ReadTool {
            name: "library_lookup",
            terminal: &["library", "lookup"],
            positional: &["name"],
            description: "A declaration of the group's Lean library, by name (Foo.bar, or \
                          lean:Foo.bar): its kind, type, docstring, module and place, whether \
                          it is proved or depends on sorry, its axioms, the declarations it uses \
                          and those that use it, and the messages and threads that reference it.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "The declaration's full name."},
                    "group": {"type": "string", "description": GROUP}
                },
                "required": ["name"]
            }),
        },
        ReadTool {
            name: "library_search",
            terminal: &["library", "search"],
            positional: &["query"],
            description: "Search the group's Lean library for declarations whose name, type or \
                          docstring contain every word of the query, regardless of case.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "The words to find."},
                    "group": {"type": "string", "description": GROUP},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100,
                              "description": "How many results; 20 if absent."}
                },
                "required": ["query"]
            }),
        },
        ReadTool {
            name: "list_papers",
            terminal: &["papers"],
            positional: &[],
            description: "List the group's working papers, the readable account of what the \
                          group is doing, each a LaTeX repository: their addresses (p/notes), \
                          titles, authors and how their PDFs stand.",
            input_schema: json!({
                "type": "object",
                "properties": {"group": {"type": "string", "description": GROUP}}
            }),
        },
        ReadTool {
            name: "read_paper",
            terminal: &["papers", "read"],
            positional: &["address"],
            description: "Read a working paper's LaTeX source, by address (p/notes), at its \
                          main: who writes it, how its PDF stands, its files, and its main \
                          file (main.tex), or the file asked for (sections/intro.tex).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "address": {"type": "string", "description": "The paper: p/notes."},
                    "file": {"type": "string",
                             "description": "The file to read, by its path in the paper; \
                                             main.tex if absent."},
                    "group": {"type": "string", "description": GROUP}
                },
                "required": ["address"]
            }),
        },
    ]
}
