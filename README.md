# xmip-core-prepare

Preparation: steps applied to a Stream without knowing its business structure
— before Message creation, during processing and before Send. A
`PreparePipeline` runs `PrepareStep`s in order; each technology under this
repository is one step, such as decoding.

A preparation step does not parse, validate or promote; it produces a Stream
from a Stream and stops. It is not a Contract and not a transformation.

`doc/architecture/runtime-model.md` section 8 governs Preparation Steps;
`architecture.toml` names the steps and carries the maturity.
