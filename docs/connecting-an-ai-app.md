# Connecting an AI app to Reeve

You can connect an AI app, such as Claude or ChatGPT, to the Reeve instance you use, so that you can ask it about your
stories, epics and decisions in a chat. The app works as you: it sees what you can see, and nothing more. An admin
has to turn this on for your instance first; if the steps below fail at the start, ask your admin.

This is for hosted apps, the ones you use in a browser or a phone app. A tool that runs on your own computer, such
as Claude Code, a desktop editor or another coding agent, does not connect this way: see
[Apps on your own computer](#apps-on-your-own-computer).

## Claude

1. In claude.ai, add a custom connector and give it the address of your instance followed by `/mcp`, for
   example `https://reeve.example.com/mcp`.
2. Claude sends you to Reeve. Sign in if you are asked to.
3. Reeve shows which app wants to connect. Before you approve: what you read through the app is sent to the app's
   provider (Anthropic for Claude), under your own account with that provider. When your instance has a privacy
   statement, this page links it. Choose **Read** or **Read and change**, then **Approve**. If you did not start this
   yourself (someone sent you the link), choose **Refuse**.
4. You are sent back to Claude, and the connector is ready.

## ChatGPT

Not yet tried against ChatGPT itself; the steps follow OpenAI's documentation and may differ in wording.

Your admin must have enabled ChatGPT for your instance; enabling Claude does not enable it. Whether you can add a
custom connector in ChatGPT depends on your ChatGPT plan and your workspace's settings.

1. In ChatGPT's settings for connectors (apps), add a custom connector and give it the address of your instance
   followed by `/mcp`, for example `https://reeve.example.com/mcp`.
2. ChatGPT should send you to Reeve. Sign in if you are asked to.
3. Reeve shows which app wants to connect. Before you approve: what you read through the app is sent to the app's
   provider (OpenAI for ChatGPT), under your own account with that provider. When your instance has a privacy
   statement, this page links it. Choose **Read** or **Read and change**, then **Approve**. If you did not start this
   yourself (someone sent you the link), choose **Refuse**.
4. You should then be sent back to ChatGPT, with the connector ready to use.

## Other apps

Other hosted assistants can be connected in the same way when your admin has enabled them: add a custom connector
with the address of your instance followed by `/mcp`. Reeve then names the app by the address it sends you back
to, and shows the name the app gives itself beside it; check that the address belongs to the app you are adding. If
such an app later says it cannot connect, remove the connector in the app and add it again.

## What the app can do

With **Read** it can search Reeve, list and open stories, epics and initiatives, list the required decisions (the
choices that must be made before work is ready), and show a sprint.

With **Read and change** it can also, as you: create and edit a story, an epic or an initiative, add a comment, and
move a story. Starting work on a story requires it to be ready; the app cannot override that check (a person can, in
the CLI or the web client), and it tells you why a story is not ready. It cannot mark an epic's or an initiative's
discovery complete, and it cannot add or remove a blocker. It can also put a story in another epic and an epic
under another initiative; that changes which discovery and which decisions the story, or the epic's stories, wait
on before work can start. If you connected an app before these were added, it can do them now without asking you
again; to take that away, revoke the connection and connect again with **Read**. To
keep a mistaken or planted instruction from doing much, one person's apps may try at most 30 changes in any ten
minutes between them; after that the app tells you when it can go on.

Whatever you choose, it can never issue tokens or change who has access.

## How long it lasts

A connection lasts thirty days from the day you approved it. Then the app asks you to connect again, and you go
through the steps above once more. If the app says it lost its connection to Reeve before that, connect again
the same way.

## Seeing and ending your connections

In Reeve, open the **Tokens** page: your apps are under **Connected apps**, and you can revoke any of them there.
From the command line:

    reeve auth connections
    reeve auth connections revoke <id>

The first lists your connections with their ids; the second ends one at once. An admin can also end a connection.

## What Reeve records

Reeve records in the activity log that you connected the app, and when a connection is revoked. Each change the app
makes is recorded as yours, together with the app it was made through: the activity feed shows it as, for example,
"commented via claude.ai".

## Apps on your own computer

Connecting is for hosted apps. A tool that runs on your own computer (Claude Code, a desktop editor, another coding
agent) is not connected this way, and asking your admin to enable it does not help. If you try, the tool reports
that it could not connect, or Reeve answers "This app runs on your own computer". Either way nothing is shared.

Use the `reeve` command-line tool there instead: such a tool can run commands. The CLI reads everything the
connection reads, and changes more: epics, initiatives, decisions and sprints, not only stories and comments.

1. Install `reeve`; your admin can say how for your instance.
2. On the Tokens page of your instance, create a token of the kind **automation**. It works as you, its changes are
   recorded as made by you through automation, and like a connected app it cannot override the readiness check. You
   can revoke it on the same page.
3. Give the tool that token and your instance's address in its environment, as `REEVE_TOKEN` and `REEVE_URL`
   (for example `https://reeve.example.com`), and ask it to use `reeve`. `reeve --help` lists the commands, and they
   take `--json`.

For your own use of `reeve` in a terminal, sign in instead: `reeve auth login --url https://reeve.example.com`.
