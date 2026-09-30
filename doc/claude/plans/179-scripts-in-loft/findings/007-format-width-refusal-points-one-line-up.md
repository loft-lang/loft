# The format-width refusal points one line above the format string
axis: clarity
met-by: fuzz/seed_program_source (strand 3)
status: open
fix: the diagnostic's span
ref: 

the refusal of `{n:0>5}` names the cure (`{n:05}`) but points at the line ABOVE the format string
