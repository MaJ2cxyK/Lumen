# Third-Party Notices

This file records the public projects that informed Lumen's product and
architecture design, the brand-asset policy, and the licensing rules that
apply to dependencies. It is a provenance record and part of the
public-repository rules in `AGENTS.md`; it is not a legal opinion.

## Projects referenced during design

The maintainer studied the projects below while planning Lumen. Lumen is an
independent implementation: the repository does not vendor their source trees,
does not list them as npm or Cargo dependencies, and ships no code, data,
fonts, or image assets copied from them. The overlap is at the level of ideas
and functional patterns, reimplemented against Lumen's own Rust, SQLite, and
React architecture.

| Project | Repository | License | Concepts referenced | Code or assets included |
|---|---|---|---|---|
| Orca | [stablyai/orca](https://github.com/stablyai/orca) | MIT | Breadth of CLI-agent adapters, usage and limit presentation, explicit "estimated" cost labeling, parallel workspaces as a later option | No |
| AionUi | [iOfficeAI/AionUi](https://github.com/iOfficeAI/AionUi) | Apache-2.0 | Registering and detecting many CLI agents, grouping agents into teams, ACP integration, mapping an Electron design onto Tauri | No |
| Herdr | [herdrdev/herdr](https://github.com/herdrdev/herdr) | Apache-2.0 | Run state model, session persistence, agent status detection, feasibility of supervising terminal processes from Rust | No |
| OpenRig | [mvschwarz/openrig](https://github.com/mvschwarz/openrig) | Apache-2.0 | Seat, team, and collaboration data model; layered permission policy; templated teams | No |
| Mission Control | [builderz-labs/mission-control](https://github.com/builderz-labs/mission-control) | MIT | Task lifecycle, approval and audit receipt concepts, and cost views as control-plane ideas (not its schema or migrations) | No |
| Craft Anything | [storytold/craft-anything](https://github.com/storytold/craft-anything) | Apache-2.0 | Staged execution with a check-and-refine loop, inline approval cards, agent-agnostic tool surface | No |
| REA | [morluto/rea](https://github.com/morluto/rea) | MIT | MCP as a common gate and tool channel between the application and an agent | No |
| dsh-superagent-prototype | [deepseek-ai/dsh-superagent-prototype](https://github.com/deepseek-ai/dsh-superagent-prototype) | MIT | Attention budget, derived stage model, task-level mandate, Git worktree isolation, restart recovery, layered prompt guidelines | No |

The license column reflects the repository's GitHub license metadata on
2026-11-26. No MIT copyright notice or Apache `NOTICE` file is required while
no source is copied. If code is ever copied from one of these projects:

- MIT code must retain its copyright and permission notice.
- Apache-2.0 code must retain copyright and `NOTICE` material and honor
  section 6, which does not grant trademark rights.
- Code with no declared license must be treated as all rights reserved and
  must not be copied.

## Brand assets

`src/components/CliLogo.tsx` renders self-drawn monochrome lettermarks: a
two-letter abbreviation on a neutral rounded tile. The current tree ships no
third-party logo artwork, icon font, brand color palette, product screenshot,
or marketing image.

Product names are used only to identify the CLI tools Lumen can launch and to
describe compatibility. All product names, trademarks, and logos remain the
property of their respective owners. Their use does not imply endorsement,
sponsorship, or affiliation with Lumen. Open-source licenses grant copyright
permissions only; they do not grant trademark rights.

If a rights holder prefers a different treatment, open an issue and the mark or
name will be removed or replaced.

Reference screenshots and mood boards used during planning are kept out of
version control and are not shipped. No third-party product screenshot is
reproduced in the application, documentation, or demo media.

Team-template names and role descriptions were rewritten in original wording
during the 2026-11-27 review; no text from those screenshots is reproduced.

## Runtime dependencies

npm and Cargo dependencies keep their own licenses. `package-lock.json` and
`src-tauri/Cargo.lock` are the authoritative dependency lists; Lumen's MIT
license does not replace or override their terms.

The dependency license audit on 2026-11-26 covered 312 npm packages and 582
Cargo packages. No package uses GPL, AGPL, or SSPL in a way that would extend
its terms to Lumen. One crate, `r-efi`, is offered under
`MIT OR Apache-2.0 OR LGPL-2.1-or-later`; Lumen relies on the permissive
option.

## Verification record

Checked on 2026-11-26 against the eight repositories above:

- Text-only checkouts were compared with 22,028 distinct normalized Lumen
  lines and 17,221 code-only lines.
- Consecutive-block detection (three or more matching lines and at least 80
  characters) found only dependency import headers and generic
  React/Tailwind boilerplate. No project-specific logic, SQL, comments, or
  user-facing strings matched.
- Directory-structure comparison found no shared layout beyond the generic
  `src/App.tsx` and `src/main.tsx` entry-point names used by Orca and OpenRig.
- The working tree and Git history contain no copyright headers, attribution
  strings, vendored directories, or dependency coordinates from those
  projects.
- The only third-party names in the product are CLI detection identifiers
  such as `Qoder CLI`, which describe external tools rather than copied
  material.

String and block comparison can show the absence of copied text; it cannot
prove that no idea was ever discussed. This record reflects the maintainer's
declared provenance and must be updated whenever a new reference, dependency,
or asset is introduced.
