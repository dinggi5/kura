// Kura MCP 어댑터 — 라이브러리 표면.
//
// 모듈을 lib 으로 노출해 바이너리(main.rs)와 통합 테스트(tests/)가 함께 쓴다.
// 비밀(니모닉/키)은 어느 모듈도 다루지 않는다 — 파일 읽기 + RPC + HTTP 만.

pub mod chain;
pub mod erc8004;
pub mod flow;
pub mod i18n;
pub mod payment;
/// ~/.jigap 파일 해석 규칙의 정본 — GUI(src-tauri)와 **같은 소스 파일**을 컴파일한다(개발 56).
/// 크레이트가 아니라 `#[path]` 모듈이라 Cargo 의존성은 그대로 0 이다. 자세한 조건은 파일 머리.
#[path = "../../shared/policy.rs"]
pub mod policy;
pub mod wallet;
pub mod x402;
