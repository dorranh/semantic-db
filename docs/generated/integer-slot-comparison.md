# Exact-width integer slot comparisons

Graph `compare_slots` accepts two operands with the same exact physical type from Int8, Int16, Int32, Int64, UInt8, UInt16, UInt32 and UInt64. Mixed widths and signedness remain rejected. Both operands must belong to the same input node and have compatible authored units. The six comparison operators preserve SQL UNKNOWN when either operand is null; the Boolean result is nullable if either input is nullable.

The implementation uses the existing native comparison expressions and generated SQL operators, without new UDFs or an execution-profile change. The interpreter prompt describes the closed type contract. Compiler pipeline35 invalidates replay/cache identity for the expanded accepted profile.

Static review found no blocking implementation defects. Tests cover all eight widths and six operators, signed extrema, UInt64MAX, null behavior, native/generated SQL parity, mixed-width/sign rejection, and existing scope/unit rejection.

Parent verification reported 261 compiler/interpreter tests across 65 targets passed (`widths-tests-final.log`) and all-targets Clippy for two packages passed (`widths-clippy.log`). CLI build passed in `/tmp/semantic-eval-comparison-widths-build.log` (27.23 seconds). This reviewer ran no Cargo commands. These checks do not establish a fresh full live Ask score; focused or full runtime reports must be retained separately.
