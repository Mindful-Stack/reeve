# Connecting an AI app to Reeve

You can connect an AI app, such as Claude, to the Reeve instance you use, so that you can ask it about your
stories, epics and decisions in a chat. The app works as you: it sees what you can see, and nothing more. An admin
has to turn this on for your instance first; if the steps below fail at the start, ask them.

## Claude

1. In claude.ai, add a custom connector and give it the address of your instance followed by `/mcp`, for
   example `https://reeve.example.com/mcp`.
2. Claude sends you to Reeve. Sign in if you are asked to.
3. Reeve shows which app wants to connect. Choose **Read** or **Read and change**, then **Approve**. If you did not
   start this yourself (someone sent you the link), choose **Refuse**.
4. You are sent back to Claude, and the connector is ready.

## What the app can do

With **Read** it can search Reeve, list and open stories, epics and initiatives, list the required decisions (the
choices that must be made before work is ready), and show a sprint.

**Read and change** will also let it create and edit stories, comment and move stories, as you. The tools that
change things arrive in a later release; until then **Read and change** works as **Read**.

Whatever you choose, it can never override the readiness gate, issue tokens or change who has access.

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

## Where what you read goes

What you read through the app is sent to the app's provider (Anthropic for Claude), under your own account with
that provider. Reeve records the app's name with each change it makes, in the activity log. When your instance has a
privacy statement, the page where you approve links it.

## ChatGPT

Not yet tried. This section will be written after its trial.
