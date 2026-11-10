---
name: plan_content_production
description: How to split, gate and accept the production of content — images, audio, video, documents, slide decks — from a brief to delivered files. Use it when the objective ends in media or documents a person will look at, especially when generation is slow or costs money.
---
# Planning content production

Three stages with gates between them: material, generation, review. Nothing expensive starts before its gate is passed.

## Material

- Collect what generation needs before generating: the brief (audience, purpose, length or duration, format, language, tone), reference files, required text, names and spellings, and what must not appear.
- For anything longer than one piece, write the structure first as its own Todo: an outline for a document, a shot list or storyboard for a video, a script for narration. Its report is the plan the generation Todos follow.
- Gate 1: the structure covers the brief, and each unit of it says what it shows or says, how long it is, and which references it uses.
- A choice only the user can make that changes most of the output (style, length) is asked before generation.

## Generation

- One Todo per unit that can be judged alone: a chapter, an image set, a clip, a narration track. Independent units run side by side; a unit that needs another's output (a clip from a keyframe, subtitles from the narration) runs after it.
- Start with one representative unit when there are many of the same kind. Gate 2: that sample is checked against the brief before the rest are generated.
- Give each generation Todo a manual for its kind of media, and put into it the unit's line of the structure, the references and the technical target (dimensions, duration, format, where to deliver).
- Each generation Todo reports the files, the request behind each, and the ids of long-running generation tasks, so a retry resumes instead of paying again.
- Assembly (joining clips, mixing audio, laying out pages) is its own Todo after its units, with a manual for that tooling.

## Review

- A review Todo that did not generate the content checks it, per unit and as a whole.
- For media, "accepted" means all of these, each observed and not assumed:
  - the file exists, opens, and has the stated format, dimensions and duration;
  - a model or a person looked at or listened to the content itself, and it shows or says what the structure asked for;
  - required text, names and numbers are present and correct; nothing forbidden appears;
  - units agree with each other: same characters, style, voice, terminology;
  - it is delivered where the brief asked, and can be opened from there.
- A generation task that ended successfully is not an accepted result.

## When things go wrong

- A unit that fails review is regenerated alone, with the reason from the review in its Todo; accepted units are not touched.
- When the requested production method is not available, report that gap; do not substitute another method unless the user agreed.
- After two attempts with the same fault, stop and report what the method cannot do.

## Accepted when

Every unit of the structure has an accepted file, the assembled result passed review as a whole, and the final report lists the delivered files, how each was checked, and what differs from the brief.
