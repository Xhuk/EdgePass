# Skills

Plain Markdown. No framework, no plugin, no harness.

A skill here is a set of instructions an AI agent can read before it touches the
project. The format is the one most coding agents already understand: a folder with
a `SKILL.md` inside, a `name`, and a `description` that says when to use it.

## Why they are here

The instructions we follow when operating edgePass are the same instructions a
stranger needs on day one. Shipping them inside the repository means:

- The agent that helps a new user knows the **prerequisites** before it guesses.
- The "never print a value" rule travels with the code.
- Backup and restore are not folklore.

## Available skills

| Skill | Use it when |
|---|---|
| [edgepass](edgepass/SKILL.md) | An agent must read, copy or inject a secret through the widget. |
| [openbao-bootstrap](openbao-bootstrap/SKILL.md) | An agent walks a human through installing, initializing, unsealing or backing up OpenBao. |
| [edgepass-led](edgepass-led/SKILL.md) | A human or an agent wants a new header LED: what to watch, how to read its state, what a click does, and how to register it without touching the exe. |

## How to use them

Give the file to your agent, or point it at the folder. A short prompt works:

> Read `skills/openbao-bootstrap/SKILL.md` and `skills/edgepass/SKILL.md`. Then guide
> me through the prerequisites on this machine.

Any agent that reads files can use them. Nothing here assumes Claude, Cursor, Codex,
Reasonix or Hermes.

## Rules that apply to every skill

1. A secret value never appears in a chat, a log, a file, or a command argument.
2. A human loads a value in the widget. An agent receives it through a child
   process environment.
3. If a value is printed, it is compromised. Rotate it, then report it.
