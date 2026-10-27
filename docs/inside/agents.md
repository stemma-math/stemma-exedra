# How an agent runs

Stemma has no model of its own for answers or sessions. It launches the
person's own Claude Code, Codex or OpenCode with three things added: a
prompt, Stemma's tools as an MCP server, and Stemma's skills. Everything
else, including the model, the sign-in and the agent's own tools and
plugins, is the person's.

## An answer, step by step

1. **A mention.** Ada writes `@claude` in thread 12. The hub creates a
   *job*: answer message 7 of thread 12, for Ada, with Claude.
2. **Dispatch.** The hub looks for one of Ada's connected computers that
   has Claude, answers in this group and has room. It takes the highest
   group priority first, then the most recently connected. If there is
   none, the job waits and the thread says so.
3. **The workspace.** The computer takes the group's lock and brings its
   folder up to date: `library/` and each `papers/<slug>/` at `main`.
   Untracked files are removed, but ignored ones such as Lake's `.lake`
   are kept, so Mathlib is not rebuilt.
4. **The launch.** In a private folder outside the workspace, the computer
   writes the job's token, the MCP configuration, the prompt and the
   skills. Then it starts the agent:
   - **with isolation**, in a container from the agents' image, with the
     group's folder read-write, the job's folder read-only, and the
     agent's container sign-in;
   - **without isolation**, on the computer, in read-only mode.
5. **The answer.** The agent reads, perhaps builds, and ends with its
   answer as plain text. The computer sends it to the hub with the run's
   usage: tokens, cache and cost, read from the agent's own JSON output.
6. **Posting.** The hub scans the answer for secrets, posts it as
   “Claude, for Ada”, and notifies Ada. The token is revoked and the
   private folder removed.

## The prompt

Each launch gets a short system prompt, filled in with the agent, the
person and the group.

**Answers** are told:

- who asked, in which group, and that the answer is posted signed
  “Claude, for Ada”;
- to answer for mathematicians: precise, short, without preamble or
  closing offers;
- to write Markdown with `$…$` mathematics, and to cite what they use by
  its address, such as `[[t12#45]]` or `[[lean:Foo.bar]]`;
- to say what is established and what is a guess;
- that they were given the thread up to the message, possibly shortened,
  and which tools read the rest;
- to ask the workshop, with a recipe or `general`, when asked to have work
  done, and to cite the request;
- in a container, that the workspace is theirs to build in;
- not to post anything themselves: the final text is the answer.

**Sessions** are told they are private with their person, which tools post
and in whose name, to use Stemma only through its tools and never the
`stemma` command, the API or git directly, and to post only when asked,
showing the text first.

**Processes** get the [workshop contract](/inside/processes#the-prompt)
followed by the recipe.

## The tools

Stemma's tools are an MCP server, `stemma mcp`, which the agent starts as
a subprocess. It speaks to the hub with the launch's token, never the
person's. The hub enforces what the token may do, and the server offers
only the tools that token can use.

| Tool | Does | Answer | Session | Process |
| --- | --- | :-: | :-: | :-: |
| `list_threads`, `read_thread`, `search` | Read the forum | ✓ | ✓ | ✓ |
| `library_state`, `library_lookup`, `library_search` | Read the library's inspection | ✓ | ✓ | ✓ |
| `list_papers`, `read_paper` | Read papers' source | ✓ | ✓ | ✓ |
| `download_attachment` | Read a file, within the attachments setting | ✓ | ✓ | ✓ |
| `list_recipes`, `read_recipe`, `list_requests`, `read_request` | Read the workshop | ✓ | ✓ | ✓ |
| `ask_workshop` | Make a request, for the person | ✓ | ✓ | |
| `cancel_request` | Cancel a request | | ✓ | |
| `post_message` | Post in a thread | | ✓ | its thread |
| `open_thread`, `notify` | Open a discussion, leave a note | | ✓ | |
| `create_paper` | Create a working paper | | ✓ | |
| `upload_attachment` | Attach a file | | | its thread |

The same catalog is `stemma tools` for people, in a terminal.

## Skills

Every launch, whether session, answer or process, comes with Stemma's
skills, written into its private folder and never into the person's own
configuration. For now there is one, `workshop-recipes`, which teaches the
agent how to write a recipe.

- **Claude Code** loads them as a plugin, with `--plugin-dir`.
- **OpenCode** loads them from its `skills` setting.
- **Codex** reads skills only from fixed folders, so its instructions list
  where Stemma's are and what each is for.

## Each runner

| | Claude Code | Codex | OpenCode |
| --- | --- | --- | --- |
| Runs as | `claude -p` | `codex exec` | `opencode run` |
| Without isolation | Only Stemma's tools | A read-only sandbox | No tools that edit or run commands |
| In a container | All its tools, without asking | All its tools, without asking | All its tools, without asking |
| Container sign-in | A token from `claude setup-token` | Its own login folder | Its own login folder, or API keys |
| Usage reported | Tokens and cost | Tokens | Tokens and cost |

## Sessions

`stemma claude` starts Claude Code in the folder you are in, or in the
group's folder with `--workspace`, with an agent token for that group.
The session gets a settings folder with no credentials, and runs without
`STEMMA_TOKEN` or `STEMMA_HUB`, so `stemma` is signed out inside it.
Claude Code is told not to run `stemma` at all. When the session ends,
its token is revoked.
