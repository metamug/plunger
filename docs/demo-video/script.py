"""The demo film's script: one entry per scene, with the words the narrator says.

`narrate.py` turns each `say` into speech with Amazon Polly (the generative voice) and measures it; `build.py`
makes each scene's picture last as long as its narration. Edit the words here, run narrate.py again, and the
film re-times itself.

Spelling for the voice: letters that are read one by one are written as <spell>MCP</spell>.
"""

VOICE = "Matthew"          # a generative voice: it reads like a person, with breath and stress
ENGINE = "generative"
REGION = "us-west-2"

SCENES = [
    {
        "id": "01_hook",
        "say": "Your A I agent just called an A P I. <break time='350ms'/> What did it send? "
               "<break time='300ms'/> And where did your token go?",
    },
    {
        "id": "02_problem",
        "say": "Agents are good at calling A P Is. But they do it with curl, and curl keeps no record. "
               "A secret lands in a log. <break time='250ms'/> A variable you forgot to define goes out as "
               "literal text. <break time='250ms'/> And afterwards, nobody can say what was actually sent.",
    },
    {
        "id": "03_meet",
        "say": "This is Plunger. <break time='300ms'/> An H T T P client built for you, and for your agent. "
               "One small program, three ways in: a window for you, a command line, and an M C P server "
               "for your agent. They share the same saved requests, the same variables, and the same history.",
    },
    {
        "id": "04_window",
        "say": "Start in the window. Paste a U R L and press send. The response comes back as a tree you can "
               "fold, search, and copy a path from. Query parameters stay in sync with the U R L, and "
               "your saved requests live in groups on the left.",
    },
    {
        "id": "04b_bodies",
        "say": "Need a body? Pick J S O N, a form, multipart with real files, or raw text. Format tidies it up, "
               "the editor never breaks a long value in half, and the response shows exactly what the server received.",
    },
    {
        "id": "04c_save",
        "say": "Save it with Control S, and file it in a group straight from its menu. Groups nest, so a big A P I "
               "stays tidy, and an agent can file its requests in the same groups.",
    },
    {
        "id": "05_variables",
        "say": "Put the parts that change in variables. Here the base address and the key are variables, so the "
               "request holds only references. <break time='300ms'/> Forget to define one, and Plunger refuses "
               "to send, and tells you which one, instead of leaking a placeholder to the server.",
    },
    {
        "id": "06_agent",
        "say": "Now the agent. From the command line, or over M C P, it sends a request and asks for only the "
               "values it needs. Here, one field out of a large response. <break time='300ms'/> "
               "The token it uses is never shown to it, and every call lands in your history, tagged C L I "
               "or M C P, as it happens.",
    },
    {
        "id": "07_workflow",
        "say": "When one response feeds the next request, save the steps as a workflow. The agent runs it in "
               "one call, and you watch it as a flow. Each step lights up, and the value one step hands to "
               "the next is written on the connector.",
    },
    {
        "id": "07b_history",
        "say": "Every request, yours or your agent's, lands in the history. Open one, and it still says how it went, "
               "and who sent it. And if you prefer a lighter look, there's a light theme, one click away.",
    },
    {
        "id": "08_safe",
        "say": "Everything an agent does is written to a tamper evident log. Credentials in headers, query "
               "strings, and request bodies are masked before anything touches the disk, and you can verify "
               "the chain at any time.",
    },
    {
        "id": "09_outro",
        "say": "Plunger is open source, and M I T licensed. Install it with pip, Homebrew, or winget, and "
               "run plunger install to wire it into your agent. <break time='400ms'/> Unclog your A P I.",
    },
]
