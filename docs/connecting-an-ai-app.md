# Connecting an AI app to Reeve

You can connect an AI app, such as Claude, to the Reeve instance you use, so that you can ask it about your
stories, epics and decisions in a chat. The app works as you: it sees what you can see, and nothing more. An admin
has to turn this on for your instance first; if the steps below fail at the start, ask your admin.

## Claude

1. In claude.ai, add a custom connector and give it the address of your instance followed by `/mcp`, for
   example `https://reeve.example.com/mcp`.
2. Claude sends you to Reeve. Sign in if you are asked to.
3. Reeve shows which app wants to connect. Before you approve: what you read through the app is sent to the app's
   provider (Anthropic for Claude), under your own account with that provider. When your instance has a privacy
   statement, this page links it. Choose **Read** or **Read and change**, then **Approve**. If you did not start this
   yourself (someone sent you the link), choose **Refuse**.
4. You are sent back to Claude, and the connector is ready.

## What the app can do

With **Read** it can search Reeve, list and open stories, epics and initiatives, list the required decisions (the
choices that must be made before work is ready), and show a sprint.

With **Read and change** it can also, as you: create a story, edit a story, add a comment, and move a story. Starting
work on a story requires it to be ready; the app cannot override that check (a person can, in the CLI or the web
client), and it tells you why a story is not ready. To
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

## ChatGPT

Not yet tried. This section will be written after its trial.
