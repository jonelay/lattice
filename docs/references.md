# References

The published methods Lattice Auditing & Tracing Tool follows, for a reader who wants to check how a result is
computed.

## Graph layout

### Sugiyama, Tagawa & Toda (1981)

*Methods for Visual Understanding of Hierarchical System Structures.*
IEEE Transactions on Systems, Man, and Cybernetics, SMC-11(2), 109–125.
https://doi.org/10.1109/TSMC.1981.4308636

The graph view in `latt browse` lays out nodes with this paper's four steps:
longest-path layer assignment, crossing reduction by the barycentric heuristic,
priority horizontal positioning, and drawing on a grid. Cycles are condensed first, and
an edge that spans several layers passes through virtual nodes, as the paper describes.
