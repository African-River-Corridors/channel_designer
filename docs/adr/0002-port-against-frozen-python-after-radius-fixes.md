---
status: accepted
---
# Port against a frozen Python reference, after fixing the silent radius failures

The port follows the OVCD method: freeze the Python package, generate reference results from it
for every entry point, and accept the Rust code only when it matches within a fixed tolerance.
The reference results are deleted once the port is done. One deliberate change from OVCD's
"identical results first, fix later" rule: #9 and #10 are fixed in Python *before* the freeze.
Both let a design line tighter than the minimum bend radius pass with no flag. Outside users
cannot check that themselves, and a frozen reference would make the wrong answer the target.
