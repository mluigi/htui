import re

with open('/home/mluigi/projects/htui/HANDOFF.md', 'r') as f:
    content = f.read()

content = content.replace(
    '**MOD-17 owns the masked DSN field inside it**',
    '**MOD-25 owns the masked DSN field inside it**'
)

# And in MOD-17 where it says "MOD-15 owns the Settings connection *section*, this item owns the credential *field* inside it"
# Since MOD-17 is superseded by MOD-25, we might leave it as is or update it? The instruction was "add it to mod 25" (Re-assign the masked DSN field to MOD-25).
# I'll update MOD-25 to mention it owns the DSN field, and also change MOD-17's text just in case.
# Let's look closely at MOD-25 and append the DSN responsibility to it.
mod_25_desc = "Not blocked. `docs/ANA-10.md` stays in the tree as the analysis that was done and not taken."
mod_25_desc_new = mod_25_desc + " **This item also owns the masked DSN field inside the Settings connection section, replacing MOD-17.**"
content = content.replace(mod_25_desc, mod_25_desc_new)

with open('/home/mluigi/projects/htui/HANDOFF.md', 'w') as f:
    f.write(content)
