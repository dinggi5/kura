// 인앱 업데이트 상태 훅 (개발 31).
//
// 정책은 전부 러스트(update.rs)에 있다 — 여기는 화면에 붙이는 얇은 껍데기다.
// 특히 **설치를 자동으로 부르지 않는다**: check 는 시작할 때 자동으로 돌지만,
// install 은 사람이 버튼을 눌러야만 불린다. 지갑에 코드를 바꿔 넣는 일이라
// "언제 바뀌었는지 모르게" 되면 안 된다.

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UpdateInfo, UpdateProgress } from "@/lib/types";

export type UpdateHook = {
  /** 찾은 업데이트. null = 없음(또는 아직 확인 안 함) */
  info: UpdateInfo | null;
  checking: boolean;
  /** 다운로드·설치가 도는 중. 성공하면 앱이 재시작되므로 이 상태로 끝난다. */
  installing: boolean;
  progress: UpdateProgress | null;
  /** 사람이 직접 누른 확인/설치에서만 채운다 — 시작 시 자동 확인은 조용히 실패한다. */
  error: string | null;
  /** 최근 확인에서 "최신입니다"가 나왔는지 (수동 확인의 피드백용) */
  upToDate: boolean;
  check: (opts?: { silent?: boolean }) => Promise<void>;
  install: () => Promise<void>;
  /** 지갑 화면 배너를 이번 실행 동안 접는다. 업데이트 자체는 버리지 않는다(설정엔 그대로 남는다). */
  bannerHidden: boolean;
  hideBanner: () => void;
};

export function useUpdate(): UpdateHook {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [upToDate, setUpToDate] = useState(false);
  const [bannerHidden, setBannerHidden] = useState(false);
  // 언마운트 뒤 늦게 도착하는 invoke 응답이 상태를 건드리지 않게.
  const alive = useRef(true);

  useEffect(() => {
    alive.current = true;
    const un = listen<UpdateProgress>("update://progress", (e) => {
      if (alive.current) setProgress(e.payload);
    });
    return () => {
      alive.current = false;
      // listen 은 Promise<UnlistenFn> — 해제도 비동기라 then 으로 받는다.
      un.then((f) => f()).catch(() => {});
    };
  }, []);

  const check = useCallback(async ({ silent = false }: { silent?: boolean } = {}) => {
    if (!alive.current) return;
    setChecking(true);
    setError(null);
    setUpToDate(false);
    try {
      const found = await invoke<UpdateInfo | null>("check_update");
      if (!alive.current) return;
      setInfo(found);
      setUpToDate(!found);
    } catch (e) {
      if (!alive.current) return;
      // 시작 시 자동 확인은 조용히 삼킨다. 네트워크가 없거나 깃허브가 잠깐 안 뜨는 건
      // 흔한 일이고, 그때마다 지갑 화면에 빨간 글씨가 뜰 이유가 없다.
      if (!silent) setError(String(e));
    } finally {
      if (alive.current) setChecking(false);
    }
  }, []);

  const install = useCallback(async () => {
    if (!alive.current) return;
    setInstalling(true);
    setError(null);
    setProgress(null);
    try {
      // 성공하면 백엔드가 앱을 재시작하므로 이 await 는 돌아오지 않는다.
      await invoke("install_update");
    } catch (e) {
      if (!alive.current) return;
      setError(String(e));
      setInstalling(false);
    }
  }, []);

  // 배너만 접는다 — info 는 그대로 둬서 설정 화면의 정보 카드에는 계속 보인다.
  const hideBanner = useCallback(() => setBannerHidden(true), []);

  return {
    info,
    checking,
    installing,
    progress,
    error,
    upToDate,
    check,
    install,
    bannerHidden,
    hideBanner,
  };
}
