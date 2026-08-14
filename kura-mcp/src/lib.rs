// Kura MCP 어댑터 — 라이브러리 표면.
//
// 모듈을 lib 으로 노출해 바이너리(main.rs)와 통합 테스트(tests/)가 함께 쓴다.
// 비밀(니모닉/키)은 어느 모듈도 다루지 않는다 — 파일 읽기 + RPC + HTTP 만.

pub mod chain;
pub mod flow;
pub mod payment;
pub mod wallet;
pub mod x402;
