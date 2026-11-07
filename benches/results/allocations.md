| Scenario | Kerniq allocs/req | Axum allocs/req | Kerniq bytes/req | Axum bytes/req |
|---|---:|---:|---:|---:|
| minimal | 13.0 | 14.0 | 1145 | 1153 |
| json | 19.0 | 20.0 | 2043 | 2027 |
| routes-50 | 19.0 | 20.0 | 2043 | 2027 |
| middleware | 34.0 | 30.0 | 5133 | 5437 |
| middleware-fused | 27.0 | 30.0 | 4365 | 5437 |
