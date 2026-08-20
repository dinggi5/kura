fn main() {
    // 신규 체크아웃 방어(코덱스 개발35 2차 P1): bundle.resources 의 kura.mcpb 는
    // 생성물이라(gitignore) 리포엔 없는데, tauri_build 는 없는 리소스에 **컴파일 자체를**
    // 실패시킨다 — 받아서 바로 `cargo test` 만 돌려도 깨진다. 빈 자리표시자로 컴파일을
    // 살린다. 빈 파일이 배포에 섞일 걱정은 없다: 번들하는 모든 경로(beforeBuild/DevCommand)
    // 는 npm run mcpb 가 진짜 파일로 먼저 교체하고, release.sh 는 앱 속 사본이 릴리스
    // 자산과 같은 바이트인지 대조한다(빈 파일이면 거기서 죽는다).
    // build-mcpb.sh 의 신선도 검사도 안 속는다 — mtime 만이 아니라 두 산출물(릴리스용·
    // 동봉본)의 **바이트 일치**까지 봐서, 자리표시자는 언제나 "다시 만들어야 함"이 된다.
    let res = std::path::Path::new("resources/kura.mcpb");
    if !res.exists() {
        let _ = std::fs::create_dir_all("resources");
        let _ = std::fs::write(res, b"");
    }
    tauri_build::build()
}
