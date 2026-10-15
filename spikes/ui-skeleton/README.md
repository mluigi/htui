# htui UI skeleton (throwaway spike)

A btop-style mock of htui's UI for trying layouts, colours and flows. **Mock data only**: no
database, no agents, no model calls. Nothing in the htui workspace depends on it, and the code is
not meant to be reused as-is: what we keep is the decision, written up as a spec.

    cargo run --manifest-path spikes/ui-skeleton/Cargo.toml

| key | what |
|-----|------|
| `1`–`6` or click | tabs: workspaces · backlog · chat · skills · logs · settings |
| `w` or click `▣` | the Workspaces tab |
| `Tab` / `Ctrl ←→` / click | move between panes (plain arrows never leave a pane) |
| `Shift ←→` / `[` `]` / click | previous / next sub-tab (works inside graphs too) |
| `p` / `P` | layout presets: IDE · IDE + activity · three columns · focus |
| `<` `>` or drag | list width · `{` `}` or drag: activity size |
| `F2` / `F3` | show/hide list / activity · `z` zooms the focused pane |
| `c` | colour scheme: tab + kind · tab only · kind only |
| `F1` | the "about this tab" panel · `?` all keys |
| `⌫` / `Alt ←` | back to where a jump (Enter in a graph, activity, requirements) came from; gone once you move |
| Backlog `N` | new item (project preset from the item on screen): describe → AI rewrite → review, or `Tab` to talk it through; `Ctrl ←→↑↓` or drag the border resizes, `F11` maximises |
| Backlog graph / runs | rataflow: arrows pick a node, `Enter` opens it (another item, the item's chat, a requirement; a run step's live chat, or its archived transcript once done), drag pans, wheel or `+` `-` `0` zoom, `f` fits, `L` legend / labels on links, `m` minimap, `d` direction, `v` runs table |
| Workspaces | `o` open (the old one keeps running in the background), `s` stop (again: force), `N` new workspace (with its folder) / project / component (type → kind → version → services → port, from the catalogue; `U` updates it) / requirement; requirements nest, `↑↓ Enter` opens a covering item, `v` shows them as a graph |
| Chat | groups: htui agent (settings, skills, catalogue) · workspace · projects · item runs · archived. Coordinators take messages (`i`); item runs are view only until they ask you |
| Activity | `↑↓` select, `Enter` jumps to the item or chat, `f` filters |
| Logs | `v` level, `F` follow, `↑↓` scroll, `G` back to the end |
| `q` / `Ctrl-C` | leave: the window closes, the work keeps running (or `S` stops it first) |

Layout, colours and the about panel are saved to `.ui-skeleton.state` on quit (delete it to reset).
`--dump <tab> [preset] [WxH] [keys]` prints one frame as text, without a terminal.
