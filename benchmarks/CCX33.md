# CPU Mark Results — CCX33 (`clickflow`)

| Detail | Value |
|--------|-------|
| **CPU Mark** | **14,694** |
| Processor | AMD EPYC-Milan Processor |
| Cores / Threads | 4 / 8 |
| RAM | 30.6 GiB |
| OS | Fedora Linux 43 (Kernel 6.18.3) |
| Integer Math | 37,689 MOps/s |
| Floating Point | 23,983 MOps/s |
| Encryption | 9,702 MB/s |
| Compression | 154,820 KB/s |
| Single Thread | 2,901 MOps/s |
| Physics | 1,911 FPS |

## Analysis

- **Overall:** A CPU Mark of ~14,700 is solid for a 4-core EPYC Milan VM. It performs on par with mid-range server workloads.
- **Integer & Floating Point:** Very strong at 37.7K and 24K MOps/s respectively — EPYC Milan's Zen 3 architecture handles math-heavy workloads well.
- **Single Thread:** 2,901 MOps/s is reasonable for Zen 3 at 4 cores, though clock speed isn't reported (likely capped by the hypervisor).
- **Encryption:** 9,702 MB/s is decent. AES hardware acceleration is working (9,064 MB/s AES), but ECDSA (7,729 MB/s) drags it slightly — typical for a virtualized environment without dedicated crypto offload.
- **Compression:** 154,820 KB/s is very strong — excellent for web/database workloads.
- **Physics:** 1,911 FPS is on the lower side, suggesting limited SIMD vectorization width in this 4-core config.
- **Note:** This is a QEMU/KVM virtual machine (QEMU DIMMs detected), so performance is bounded by the hypervisor's CPU allocation. The real host CPU likely has significantly more headroom.
