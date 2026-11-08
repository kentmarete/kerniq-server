| Scenario | Kerniq allocs/req | Axum allocs/req | Kerniq bytes/req | Axum bytes/req |
|---|---:|---:|---:|---:|
| minimal | 13.0 | 14.0 | 1145 | 1153 |
| json | 19.0 | 20.0 | 2043 | 2027 |
| routes-50 | 19.0 | 20.0 | 2043 | 2027 |
| middleware | 34.0 | 30.0 | 5133 | 5437 |
| middleware-fused | 27.0 | 30.0 | 4365 | 5437 |

## Observability (`alloc-count`, after the optimizations in docs/observability/performance.md)

| Observability | /users/{id} allocs/req | bytes/req | /stream allocs/req | bytes/req |
|---|---:|---:|---:|---:|
| baseline | 17.0 | 1886 | 16.0 | 8429 |
| metrics | 20.0 | 2654 | 19.0 | 9197 |
| traces-100% | 49.0 | 6448 | 41.0 | 12105 |
| traces-10% | 49.0 | 6133 | 41.0 | 11773 |
| json-logs | 31.0 | 3561 | 28.0 | 9912 |
| log-export | 23.0 | 3206 | 19.0 | 9197 |
| all | 63.1 | 7987 | 51.0 | 12944 |
