---
title: "Google acaba de MEJORAR Claude Code (y es GRATIS)"
source: youtube
url: "https://youtube.com/watch?v=w8iwtJyTELA&si=27f5HnqaCk5J8Ag_"
created: 2026-11-15
tags:
  - EGM
  - Claude
  - Elephant
  - Google
---

# Google acaba de MEJORAR Claude Code (y es GRATIS)

[Ver en YouTube](https://youtube.com/watch?v=w8iwtJyTELA&si=27f5HnqaCk5J8Ag_)
https://youtube.com/watch?v=w8iwtJyTELA&si=27f5HnqaCk5J8Ag_

## Resumen

This video explores the Elephant Goldfish Model (EGM), a new method introduced by Google for building applications with artificial intelligence. The primary goal of EGM is to detect potential design flaws, logic errors, and missing requirements before the actual construction of an application begins (0:00 - 1:00).

How EGM Works:

The Elephant: An AI agent that maintains the project context, understands user requirements, and drafts the initial design document (1:24 - 1:35).
The Goldfish (Fishes): Independent AI agents that review the design without having seen the original conversation. They act as a critical sounding board to identify ambiguities, contradictions, or overly complex designs (1:51 - 2:15).
The Process: Users follow a structured workflow involving commands like `draft` for planning, `review` for critique, and `resolve` for applying fixes (3:40 - 16:25).

Key Takeaways from the Comparison Test:

The video compares building a paddle club reservation app with and without EGM. While building directly (without EGM) is faster for simple prototypes, the EGM-built version proved more robust (17:00 - 18:18).
EGM Benefits: The EGM process caught critical server-side validation issues and error-handling flaws that were missed when simply asking an AI to "build the app" (17:15 - 17:42).
Recommendation: The creator suggests using EGM for complex, critical projects where accuracy is vital, while standard prompt-based building is often sufficient for quick proofs-of-concept (18:22 - 19:10).

## how to use EGM

The Elephant Goldfish Model (EGM) is a methodology designed to improve application development by introducing a structured planning and review phase before code generation. You can implement this method by installing it as a plugin in tools like Claude Code (02:30-03:20).

Step-by-Step Workflow

1.  Preparation (Drafting the Plan):

Open your project folder and use the command `/egm` to initialize the plugin (03:41).
Use the `draft` capability to provide your initial project requirements. The "Elephant" agent will then ask you specific questions to define the design and clarify ambiguities (05:04-06:15).

2.  Reviewing (The "Fishes"):

Use the `review` capability, pointing it to your design document (e.g., `design.md`). Independent agents ("Fishes") will critique the plan without access to your original conversation to identify missing requirements, logic flaws, or contradictions (10:43-11:15).

3.  Refinement (Resolving Issues):

Use the `resolve` capability to evaluate the findings from the review agents and apply necessary corrections to your design document (13:17-13:45).

4.  Construction:

Once the design document is finalized and approved, instruct the agent to build the application based on the verified plan (14:43-15:00).

Key Commands

`brainstorm`: To refine undefined ideas.
`draft`: To create the initial design document.
`review`: To perform a critical analysis of the design.
`resolve`: To apply fixes based on feedback.
`drift_report`: To compare the final code against the original plan.

## How to install EGM

Installing the Elephant Goldfish Model (EGM) plugin for tools like Claude is a straightforward process. Here is your step-by-step guide to getting it set up (2:30 - 3:51):

1.  Download the Plugin: Visit the project's official download page. Look for the version marked with a green latest label. Expand the Assets list and download the zip file prepared for your specific application (e.g., EGM Claude 0.1.1) (2:42 - 3:03).
2.  Install in the Application: Open your Claude desktop application. Navigate to Personalizar (Customize) and then select Plugins. Click Añadir (Add) and choose Subir plugin (Upload plugin). Select the zip file you just downloaded without unzipping it, and confirm the installation (3:07 - 3:20).
3.  Initialize the Project: Go to the Code tab and open a new session. Select your local computer environment and create or choose a specific folder for your project (e.g., named EGM) (3:22 - 3:41).
4.  Verify Installation: In your new project session, type the command `/egm`. If installed correctly, you will see a list of seven available EGM abilities, such as `draft` for planning and `review` for checking (3:41 - 3:51).