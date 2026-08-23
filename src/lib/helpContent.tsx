// 도움말 콘텐츠 — 인앱 도움말 화면(HelpScreen)과 첫 실행 환영 투어(WelcomeTour)가 공유하는 한 벌.
// "사람 말로" 원칙: 전문용어 최소화, 처음 켠 사람이 바로 이해하게.
//
// 두 언어를 한 섹션 안에 나란히 둔다(개발 42) — 문서 성격의 글이라 한쪽만 고치면 금세 어긋난다.
// 번역이 아니라 **같은 말을 그 언어로** 쓴다: 영어 쪽도 짧은 문장, 조용한 톤, 느낌표 없이.

import { ArrowDownLeft, Bot, KeyRound, Plug, ShieldCheck, Stamp } from "lucide-react";

import { t } from "./i18n";

/** 배포 문서·정확한 설정값이 있는 곳. 공개 저장소라 누구나 열린다(개발 29). */
export const GITHUB_URL = "https://github.com/dinggi5/kura";

export type HelpSection = {
  id: string;
  icon: React.ReactNode;
  title: string;
  /** 도움말 화면·투어에 그대로 들어가는 본문. */
  body: React.ReactNode;
};

const em = "text-[var(--color-ink-700)] dark:text-[#E8E5DD]";

export const HELP_SECTIONS: HelpSection[] = [
  {
    id: "what",
    icon: <Bot size={22} />,
    title: t("Kura가 뭐예요?", "What is Kura?"),
    body: t(
      <div className="space-y-2.5">
        <p>
          Kura는 <b className={em}>AI 에이전트를 위한 지갑</b>이에요. Claude 같은 AI가
          인터넷에서 뭔가를 결제해야 할 때, 당신 대신 결제를 <b className={em}>요청</b>하고,
          당신이 <b className={em}>비밀번호로 승인</b>하면 실행돼요.
        </p>
        <p>
          돈을 움직이는 열쇠(키)는 <b className={em}>이 컴퓨터를 절대 떠나지 않아요.</b>{" "}
          클라우드에 올라가지 않고, 채팅창에도 들어가지 않아요.
        </p>
        <p>
          Kura는 <b className={em}>메뉴바에 상주</b>해요. 화면 위쪽 메뉴바의 열쇠구멍 아이콘을
          누르면 열리고, 다른 곳을 클릭하면 닫혀요. 닫아도 백그라운드에서 결제 요청을 계속
          기다리니, 필요할 때 알아서 앞으로 나와요.
        </p>
      </div>,
      <div className="space-y-2.5">
        <p>
          Kura is <b className={em}>a wallet for AI agents</b>. When an AI like Claude needs to
          pay for something online, it <b className={em}>asks</b> instead of paying, and the
          payment goes through once you <b className={em}>approve it with your password</b>.
        </p>
        <p>
          The key that moves the money <b className={em}>never leaves this computer.</b> It is
          never uploaded to a cloud, and it never enters a chat window.
        </p>
        <p>
          Kura <b className={em}>lives in your menu bar</b>. Click the keyhole icon up top to
          open it, click anywhere else to close it. Closing it doesn't stop anything — it keeps
          waiting for payment requests in the background and comes forward when it needs you.
        </p>
      </div>,
    ),
  },
  {
    id: "fund",
    icon: <ArrowDownLeft size={22} />,
    title: t("돈은 어떻게 채워요?", "How do I add money?"),
    body: t(
      <div className="space-y-2.5">
        <p>
          결제하려면 지갑에 <b className={em}>USDC</b>(디지털 달러)가 있어야 해요.{" "}
          <b className={em}>받기</b> 버튼을 누르면 당신 지갑 주소와 QR이 나와요.
        </p>
        <p>
          거래소나 다른 지갑에서 이 주소로 보낼 때는 <b className={em}>반드시 Base 네트워크</b>
          를 고르세요. 다른 네트워크(이더리움 등)로 보내면 자금을 잃을 수 있어요.
        </p>
        <p>
          기본 설정인 <b className={em}>테스트넷</b>에선 진짜 돈이 아니라서, 받기 화면의 Faucet
          버튼으로 공짜 테스트 코인을 받아 연습할 수 있어요.
        </p>
        <p>
          <b className={em}>결제엔 ETH가 없어도 돼요</b> — x402 결제의 가스비는 대신 내주는
          구조예요. 보내기로 직접 송금할 때만 가스용 ETH가 아주 조금 필요해요.
        </p>
      </div>,
      <div className="space-y-2.5">
        <p>
          To pay for anything, the wallet needs <b className={em}>USDC</b> — a digital dollar.
          Tap <b className={em}>Receive</b> to see your address and its QR code.
        </p>
        <p>
          When you send from an exchange or another wallet, you must pick the{" "}
          <b className={em}>Base network</b>. Sending over a different network (Ethereum, for
          example) can lose the funds.
        </p>
        <p>
          On <b className={em}>the testnet</b> nothing is real money, so you can practice with
          free test coins from the Faucet buttons on the Receive screen.
        </p>
        <p>
          <b className={em}>You don't need ETH to pay</b> — with x402 the gas is covered for you.
          Only a direct transfer from the Send screen needs a tiny amount of ETH for gas.
        </p>
      </div>,
    ),
  },
  {
    id: "connect",
    icon: <Plug size={22} />,
    title: t("AI(Claude 등)랑 연결하기", "Connecting an AI"),
    body: t(
      <div className="space-y-2.5">
        <p>
          <b className={em}>AI에게 Kura를 한 번만 소개해주면</b> 그다음부턴 알아서 연결돼요.
          (이 "소개"를 MCP라고 불러요 — AI 앱이 지갑 같은 바깥 도구를 쓰게 해주는 표준
          연결이에요.)
        </p>
        <p>
          메인 화면 위쪽의 <b className={em}>"AI 연결 안 됨" 배지를 누르면</b> 연결 화면이
          열려요. 거기서 버튼 한 번이면 돼요:
        </p>
        <ul className="space-y-1.5">
          <li>
            <b className={em}>Claude 데스크톱</b> — "연결" 버튼을 누르면 Claude에 확장 설치
            창이 떠요. '설치'만 누르면 끝.
          </li>
          <li>
            <b className={em}>Claude Code</b> — 버튼 한 번으로 등록돼요. 다음 claude 실행부터
            어느 폴더에서든 연결돼요.
          </li>
        </ul>
        <p>
          연결되면 배지가 <b className={em}>"Claude 연결됨"</b>으로 바뀌어요. 그때부터 AI가
          결제를 요청하면 승인 팝업이 떠요.
        </p>
        <p className="text-[var(--color-ink-300)]">
          Cursor 같은 다른 앱도 연결 화면의 서버 경로로 붙일 수 있어요. 자세한 건 GitHub 문서에.
        </p>
      </div>,
      <div className="space-y-2.5">
        <p>
          <b className={em}>Introduce Kura to your AI once</b> and it connects on its own after
          that. (That introduction is called MCP — the standard way an AI app reaches outside
          tools like a wallet.)
        </p>
        <p>
          Tap the <b className={em}>"No AI connected" badge</b> at the top of the main screen to
          open the connect screen. One button is all it takes:
        </p>
        <ul className="space-y-1.5">
          <li>
            <b className={em}>Claude desktop</b> — press "Connect" and Claude opens its extension
            installer. Press Install and you're done.
          </li>
          <li>
            <b className={em}>Claude Code</b> — one button registers it. From the next{" "}
            <code>claude</code> run on, it's available in any folder.
          </li>
        </ul>
        <p>
          Once connected, the badge reads <b className={em}>"Claude connected"</b>. From then on,
          an approval window opens whenever the AI asks to pay.
        </p>
        <p className="text-[var(--color-ink-300)]">
          Other apps like Cursor can use the server path shown on the connect screen. The GitHub
          docs have the details.
        </p>
      </div>,
    ),
  },
  {
    id: "flow",
    icon: <Stamp size={22} />,
    title: t("결제는 이렇게 흘러가요", "How a payment goes"),
    body: t(
      <ol className="space-y-1.5">
        <li>
          <b className={em}>①</b> AI가 결제를 요청해요.
        </li>
        <li>
          <b className={em}>②</b> Kura 창이 떠서 <b className={em}>얼마를, 어디로</b> 보내는지
          보여줘요.
        </li>
        <li>
          <b className={em}>③</b> 당신이 비밀번호를 넣고 승인해요.
        </li>
        <li>
          <b className={em}>④</b> 결제가 실행되고, 결과가 AI에게 돌아가요.
        </li>
        <li className="pt-1 text-[var(--color-ink-300)]">
          5분 안에 승인하지 않으면 자동으로 거부돼요.
        </li>
      </ol>,
      <ol className="space-y-1.5">
        <li>
          <b className={em}>①</b> The AI asks to pay.
        </li>
        <li>
          <b className={em}>②</b> Kura opens and shows you <b className={em}>how much, to whom</b>
          .
        </li>
        <li>
          <b className={em}>③</b> You type your password and approve.
        </li>
        <li>
          <b className={em}>④</b> The payment goes out, and the result goes back to the AI.
        </li>
        <li className="pt-1 text-[var(--color-ink-300)]">
          If you don't approve within five minutes, it's rejected automatically.
        </li>
      </ol>,
    ),
  },
  {
    id: "safety",
    icon: <ShieldCheck size={22} />,
    title: t("안전장치", "What keeps you safe"),
    body: t(
      <ul className="space-y-2">
        <li>
          <b className={em}>비밀번호 승인</b> — 기본값은 매 결제마다 당신이 비밀번호를 넣어야
          실행돼요. (아래 자율 결제를 켠 경우만 예외)
        </li>
        <li>
          <b className={em}>한도</b> — 한 번에 / 하루에 얼마까지 보낼지 설정에서 정해요. 넘으면
          막혀요.
        </li>
        <li>
          <b className={em}>긴급 잠금</b> — 헤더의 방패 버튼을 켜면 모든 송금이 즉시 막혀요.
        </li>
        <li>
          <b className={em}>신뢰 주소 · 자율 결제</b> — 비번 없이 자동 승인하려면 기본 설정에선
          세션 잠금 해제 + 소액 한도 + 신뢰하는 주소, <b className={em}>셋 다 맞을 때만</b>{" "}
          돼요.
        </li>
      </ul>,
      <ul className="space-y-2">
        <li>
          <b className={em}>Password approval</b> — by default every payment waits for your
          password. (Autopay, below, is the one exception.)
        </li>
        <li>
          <b className={em}>Limits</b> — you set how much can go out per payment and per day.
          Anything over that is blocked.
        </li>
        <li>
          <b className={em}>Emergency lock</b> — the shield button in the header blocks every
          payment at once.
        </li>
        <li>
          <b className={em}>Trusted addresses · autopay</b> — for a payment to go through without
          your password, the default setup needs all three: an unlocked session, an amount under
          the small autopay limit, and an address you've approved before.
        </li>
      </ul>,
    ),
  },
  {
    id: "backup",
    icon: <KeyRound size={22} />,
    title: t("시드 백업 (제일 중요)", "Back up your 12 words (most important)"),
    body: t(
      <div className="space-y-2.5">
        <p>
          12개 단어가 자산의 <b className={em}>진짜 소유 증명</b>이에요. 비밀번호를 잊으면 이
          12단어로만 자산을 되찾을 수 있어요.
        </p>
        <p>
          <b className={em}>비밀번호는 복구 수단이 아니에요</b> — 잊으면 이 지갑 파일은 다시 못
          열어요. 그래도 12단어가 있으면 자산은 되찾을 수 있어요: 지갑 파일을 지워 처음 상태로
          되돌린 뒤 첫 화면의 <b className={em}>가져오기</b>를 쓰거나 (방법은 GitHub 문서),
          다른 표준 지갑(BIP-39)에 넣으면 돼요.
        </p>
        <p>
          <b className={em}>12단어 없이 지갑 파일을 지우면 자산을 영영 잃어요.</b> 종이나
          비밀번호 관리자에 안전하게 적어두세요.
        </p>
        <p className="text-[var(--color-ink-300)]">
          헤더의 열쇠 버튼에서 언제든 다시 볼 수 있어요.
        </p>
      </div>,
      <div className="space-y-2.5">
        <p>
          Those twelve words are <b className={em}>the real proof that the funds are yours</b>. If
          you forget your password, they are the only way back to them.
        </p>
        <p>
          <b className={em}>A password is not a recovery method</b> — forget it and this wallet
          file stays closed for good. The funds are still recoverable with the twelve words:
          delete the wallet file to start fresh and use <b className={em}>Import</b> on the first
          screen (the GitHub docs show how), or restore them in any standard BIP-39 wallet.
        </p>
        <p>
          <b className={em}>Delete the wallet file without those words and the funds are gone for
          good.</b> Write them on paper or keep them in a password manager.
        </p>
        <p className="text-[var(--color-ink-300)]">
          The key button in the header shows them again any time.
        </p>
      </div>,
    ),
  },
];
