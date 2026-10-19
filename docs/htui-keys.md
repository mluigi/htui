# htui keys

Every key htui binds by default, exactly as `htui --default-keys --print-keys` prints them: a
complete `keys.toml`. To change a key, copy its line into `keys.toml` in htui's configuration
directory and edit it there; list only what you change (README, [Changing keys](../README.md#changing-keys)).

A `[global]` chord is reserved on every screen that offers it: binding it to an action in a
narrower table fails validation (htui exits with status 2 and names the clash and the screen). For
example `global.about` (the about panel) owns `f9`, so `[list] down = ["f9"]` is refused: move that
action to a free chord, or rebind the global action first.

`Esc` is never configurable: a key file cannot list it, and every row marked `Esc always` keeps it
whatever its list says (`[]` unbinds only the row's other chords). A `version = 1` file that lists
`Esc` on such a row still loads. `ctrl-c` binds like any key: by default the first press opens the
leave dialog (`global.quit`) and a second one leaves (`leave.confirm`). htui refuses a file that
leaves `global.quit` or `leave.confirm` with no chord, and likewise `leave.stay`, `field.cancel`,
`overlay.close` and `editor.focus`.

This page is generated. After a change to the key catalogue, replace the block below with the
output of `cargo run -q -p htui -- --default-keys --print-keys`; `cargo test -p htui --test
keys_file` fails until it matches.

```toml
# htui key bindings, as `htui --print-keys` prints them. htui reads keys.toml in its config
# directory: ~/.config/htui on Linux, ~/Library/Application Support/htui on macOS, %APPDATA%\htui
# on Windows. List only what you change: a list replaces that action's chords, [] unbinds it.
# Esc is never listed: a row marked "Esc always" keeps it whatever its list says. ctrl-c binds
# like any key. `htui --default-keys` ignores the file for one run.
version = 2

[global]
quit         = ["q", "ctrl-c"]           # quit
next_tab     = ["ctrl-pgdn"]             # next tab
prev_tab     = ["ctrl-pgup"]             # previous tab
select_tab_1 = ["1"]                     # select tab
select_tab_2 = ["2"]                     # select tab
select_tab_3 = ["3"]                     # select tab
select_tab_4 = ["4"]                     # select tab
select_tab_5 = ["5"]                     # select tab
select_tab_6 = ["6"]                     # select tab
select_tab_7 = ["7"]                     # select tab
select_tab_8 = ["8"]                     # select tab
select_tab_9 = ["9"]                     # select tab
help         = ["?", "f1"]               # help
about        = ["f9"]                    # about
workspaces   = ["w"]                     # workspaces
find         = ["ctrl-f"]                # find
waiting      = ["ctrl-w"]                # waiting
queue        = ["ctrl-q"]                # queue
next_pane    = ["tab", "ctrl-right"]     # next pane
prev_pane    = ["backtab", "ctrl-left"]  # previous pane

[overlay]
close = []  # close; Esc always

[list]
down   = ["j", "down"]  # down
up     = ["k", "up"]    # up
top    = ["g", "home"]  # top
bottom = ["G", "end"]   # bottom
fold   = ["enter"]      # fold

[pane]
scroll_down = ["J"]                      # scroll down
scroll_up   = ["K"]                      # scroll up
page_down   = ["pgdn"]                   # page down
page_up     = ["pgup"]                   # page up
next_subtab = ["l", "]", "shift-right"]  # next sub-tab
prev_subtab = ["h", "[", "shift-left"]   # previous sub-tab

[confirm]
yes = ["y"]  # yes
no  = ["n"]  # no; Esc always

[form]
next_field      = ["tab"]      # next field
prev_field      = ["backtab"]  # previous field
save            = ["ctrl-s"]   # save
external_editor = ["ctrl-e"]   # $EDITOR

[common]
edit    = ["e"]  # edit
new     = ["n"]  # new
delete  = ["d"]  # delete
clear   = ["c"]  # clear
reload  = ["r"]  # reload
back    = []     # back; Esc always
dismiss = []     # dismiss; Esc always

[editor]
focus = ["ctrl-4"]  # editor focus
abort = ["ctrl-x"]  # abort edit

[settings]
next_section = ["l", "]", "shift-right"]  # next section
prev_section = ["h", "[", "shift-left"]   # previous section

[settings.agents]
probe          = ["r"]              # probe
install        = ["i"]              # install
authenticate   = ["a"]              # authenticate
switch_box     = ["t"]              # this box
edit_paths     = ["m"]              # paths
open_link      = ["o"]              # open link
paste_redirect = ["p"]              # paste redirect
cancel         = ["x"]              # cancel
choose         = ["enter"]          # select
next_field     = ["tab", "down"]    # next field
prev_field     = ["backtab", "up"]  # previous field

[settings.hierarchy]
new_workspace = ["N"]              # new workspace
primary       = ["p"]              # make primary
choose_path   = ["b"]              # choose path
infer         = ["i"]              # infer paths
next_field    = ["tab", "down"]    # next field
prev_field    = ["backtab", "up"]  # previous field

[settings.kinds]
new_graph  = ["N"]              # new graph
graph      = ["g"]              # edit graph
next_field = ["tab", "down"]    # next field
prev_field = ["backtab", "up"]  # previous field

[settings.connection]
rebuild  = ["R"]      # rebuild cache
activate = ["enter"]  # run row

[settings.boxes]
edit_tags   = ["t"]  # edit tags
edit_quirks = ["e"]  # edit quirks
executor    = ["w"]  # executor
probe       = ["p"]  # probe
edit_spec   = ["s"]  # edit probe spec

[settings.personas]
body       = ["b"]              # edit body
rules      = ["r"]              # edit rules
import     = ["I"]              # import
next_field = ["tab", "down"]    # next field
prev_field = ["backtab", "up"]  # previous field
back       = ["enter"]          # back; Esc always

[settings.secrets]
check      = ["t"]              # check
next_field = ["tab", "down"]    # next field
prev_field = ["backtab", "up"]  # previous field

[settings.queue]
edit = ["e", "enter"]  # edit

[concepts]
decisions = ["ctrl-d"]  # decisions only
project   = ["ctrl-p"]  # cycle project scope
reindex   = ["ctrl-r"]  # re-index scope
up        = ["up"]      # previous hit
down      = ["down"]    # next hit

[switcher]
switch = ["enter"]  # switch workspace

[migration]
yes = ["y", "Y"]  # yes
no  = ["n", "N"]  # no; Esc always

[waiting]
open = ["enter"]  # open step

[skills]
switch_view     = ["h", "l", "[", "]", "shift-left", "shift-right"]  # switch view
prev_version    = [","]                                              # older version
next_version    = ["."]                                              # newer version
base            = ["b"]                                              # diff base
diff            = ["d"]                                              # diff
edit_externally = ["E"]                                              # edit in $EDITOR
ask_agent       = ["ctrl-g"]                                         # ask agent

[skills.library]
import     = ["I"]              # import
info       = ["i"]              # rename
attach     = ["a"]              # attachments
next_field = ["tab", "down"]    # next field
prev_field = ["backtab", "up"]  # previous field

[skills.templates]
diff_default = ["D"]  # diff default

[skills.attach]
choose     = ["enter"]          # choose row
detach     = ["x"]              # detach
repo       = ["ctrl-r"]         # repo picker
next_field = ["tab", "down"]    # next field
prev_field = ["backtab", "up"]  # previous field

[skills.help]
prev_agent = ["up"]          # previous agent
next_agent = ["down"]        # next agent
accept     = ["enter", "y"]  # accept
cancel     = []              # cancel the turn; Esc always

[requirements]
new_area       = ["a"]              # new area
amend          = ["e"]              # amend
withdraw       = ["W"]              # withdraw
filter         = ["/"]              # filter
priority_must  = ["m"]              # priority must
priority_later = ["l"]              # priority later
next_field     = ["tab", "down"]    # next field
prev_field     = ["backtab", "up"]  # previous field

[backlog]
replay       = ["enter"]  # replay step
open_graph   = ["m"]      # open graph
filter       = ["f"]      # filter
clear_filter = ["F"]      # clear filter
new_item     = ["N"]      # new item
queue        = ["Q"]      # queue / dequeue
pause_queue  = ["P"]      # pause / resume queue

[backlog.runs]
next_step   = ["J"]       # next step
prev_step   = ["K"]       # previous step
approve     = ["a"]       # approve
reject      = ["x"]       # reject
retry       = ["r"]       # retry step
promote     = ["p"]       # promote to chat
select      = ["s"]       # select winner
accept      = ["A"]       # accept output
open        = ["o"]       # open output
cancel_run  = ["c"]       # cancel run
cleanup     = ["T"]       # retry cleanup
unblock     = ["u"]       # unblock
start_run   = ["R"]       # start run
close_out   = ["C"]       # close out
follow_up   = ["i"]       # follow up
toggle_view = ["v"]       # list / flow
zoom_in     = ["+"]       # zoom in
zoom_out    = ["-"]       # zoom out
zoom_fit    = ["="]       # fit the run
scroll_down = ["J", "j"]  # scroll down
scroll_up   = ["K", "k"]  # scroll up

[backlog.graph]
next_row  = ["J"]      # next row
prev_row  = ["K"]      # previous row
deeper    = ["+"]      # deeper
shallower = ["-"]      # shallower
reroot    = ["enter"]  # re-root

[backlog.docs]
next_row    = ["J"]              # next row
prev_row    = ["K"]              # previous row
add         = ["a"]              # new document
new_version = ["v"]              # new version
next_field  = ["tab", "down"]    # next field
prev_field  = ["backtab", "up"]  # previous field

[backlog.notes]
add = ["a"]  # add note

[backlog.prompt]
next_template = ["n"]  # next template
prev_template = ["p"]  # previous template

[backlog.reqs]
next_row  = ["J"]      # next row
prev_row  = ["K"]      # previous row
reconfirm = ["r"]      # reconfirm
cite      = ["c"]      # cite
uncite    = ["u"]      # uncite
pick      = ["enter"]  # pick
addresses = ["a"]      # addresses
reserves  = ["v"]      # reserves

[backlog.filter_form]
next_row   = ["j"]              # next row
prev_row   = ["k"]              # previous row
clear      = ["x"]              # clear
next_field = ["tab", "down"]    # next field
prev_field = ["backtab", "up"]  # previous field

[backlog.item]
theirs      = ["t", "T"]          # theirs wins
mine        = ["m", "M"]          # mine wins
switch_diff = ["tab", "backtab"]  # body/paths
next_field  = ["tab", "down"]     # next field
prev_field  = ["backtab", "up"]   # previous field

[chat]
compose    = ["i", "enter"]  # compose
next_agent = ["a"]           # next agent
end        = []              # end chat (twice); Esc always
thoughts   = ["t"]           # thoughts
top        = ["g"]           # top
bottom     = ["G"]           # bottom

[queue]
move_down = ["J"]      # move down
move_up   = ["K"]      # move up
pause     = ["P"]      # pause / resume
dequeue   = ["Q"]      # dequeue
open      = ["enter"]  # open

[picker]
open       = ["enter", "l"]      # open directory
parent     = ["h", "backspace"]  # parent directory
choose     = ["s"]               # choose
choose_dir = ["S"]               # choose this directory
go_to      = ["/"]               # go to path
hidden     = ["."]               # show hidden

[layout]
next_preset     = ["p"]               # next layout
prev_preset     = ["P"]               # previous layout
grow_list       = [">"]               # wider list
shrink_list     = ["<"]               # narrower list
grow_activity   = ["}", "ctrl-down"]  # larger activity
shrink_activity = ["{", "ctrl-up"]    # smaller activity
toggle_list     = ["f2", "alt-1"]     # show/hide list
toggle_activity = ["f3", "alt-2"]     # show/hide activity
zoom            = ["z"]               # zoom

[history]
back    = ["backspace", "alt-left"]  # back
forward = ["alt-right"]              # forward

[leave]
confirm = ["enter", "ctrl-c"]  # leave
stay    = ["n"]                # stay; Esc always

[field]
submit         = ["enter"]      # submit
cancel         = []             # cancel; Esc always
left           = ["left"]       # cursor left
right          = ["right"]      # cursor right
home           = ["home"]       # line start
end            = ["end"]        # line end
delete_back    = ["backspace"]  # delete back
delete_forward = ["delete"]     # delete forward
line_up        = ["up"]         # line up
line_down      = ["down"]       # line down
page_up        = ["pgup"]       # page up
page_down      = ["pgdn"]       # page down

[choice]
prev   = ["left", "h"]   # previous option
next   = ["right", "l"]  # next option
toggle = ["space"]       # toggle

[answer]
option_1 = ["1"]  # answer
option_2 = ["2"]  # answer
option_3 = ["3"]  # answer
option_4 = ["4"]  # answer
option_5 = ["5"]  # answer
option_6 = ["6"]  # answer
option_7 = ["7"]  # answer
option_8 = ["8"]  # answer
option_9 = ["9"]  # answer
```
