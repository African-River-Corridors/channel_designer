---
status: accepted
---
# Water level belongs to the river

A channel design holds the water level as a profile along the river centreline. A design line's
station takes its level by projecting onto the river centreline. Two design lines through the same
place then see the same water, which a profile along each line's own chainage cannot give.

The frozen Python reads the level at the design line's own chainage. So the shipped results differ
from spec 0001 slightly (about 2.5e-4 relative on the quickstart's dredge), by design. A test-only
mode keeps the Python's sampling, so the port itself is still proven to 1e-9. No ADR 0005 tolerance
changes.
