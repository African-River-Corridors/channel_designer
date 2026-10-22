- **The required `test` check runs in parallel parts (contributors).** CI runs the gate as `gate-lint`,
  three `gate-rust` partitions and `gate-python` at once; the `test` check passes only when all of them
  pass. `tools/gates.sh` takes `GATES_STEPS` and `GATES_PARTITION` to run one part, and with neither it
  runs the whole gate, as before.
