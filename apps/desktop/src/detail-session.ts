import { type MutableRefObject, useRef, useState } from "react"

import {
  bridgeFailureKind,
  hydrateDetail,
  readDetail,
  sameSearchSelection,
  type DetailBody,
  type SearchHit,
} from "./daemon"
import {
  captureDaemonActionAuthority,
  daemonActionAuthorityIsCurrent,
  detailReadMayContinue,
  type DaemonActionAuthority,
  type DaemonActionAuthorityToken,
  type DaemonLifecycleSnapshot,
  type DaemonService,
} from "./runtime-state"

export const MAX_DETAIL_PAGES = 128

export type DetailViewDocument = DetailBody["document"] & { file_name: string }

interface DetailContinuation {
  hit: SearchHit
  authorityEpoch: number
  generation: number
  nextOffset: number
  text: string
  pagesRead: number
  metadataLoaded: boolean
}

export function useDetailSession(input: {
  preview: boolean
  authorityRef: MutableRefObject<DaemonActionAuthority>
  lifecycleRef: MutableRefObject<DaemonLifecycleSnapshot>
  service: DaemonService
  isCapabilityAuthorized: () => boolean
  onStaleSelection: () => void
}) {
  const [detail, setDetail] = useState<DetailViewDocument | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [detailError, setDetailError] = useState("")
  const [fullText, setFullText] = useState("")
  const [bodyComplete, setBodyComplete] = useState(false)
  const [detailInterrupted, setDetailInterrupted] = useState(false)
  const continuationRef = useRef<DetailContinuation | null>(null)
  const runIdRef = useRef(0)

  const reset = () => {
    runIdRef.current += 1
    continuationRef.current = null
    setDetail(null)
    setDetailLoading(false)
    setDetailError("")
    setDetailInterrupted(false)
    setFullText("")
    setBodyComplete(false)
  }

  const observeLifecycle = (snapshot: DaemonLifecycleSnapshot) => {
    const continuation = continuationRef.current
    if (!continuation || detailReadMayContinue(snapshot, continuation.generation)) return
    runIdRef.current += 1
    setDetailLoading(false)
    setDetailInterrupted(true)
    setDetailError("daemon 恢复中，已保留已读取内容；恢复后请显式续读")
  }

  const authorityTokenFor = (continuation: DetailContinuation): DaemonActionAuthorityToken => ({
    epoch: continuation.authorityEpoch,
    generation: continuation.generation,
  })

  const authorityIsCurrent = (token: DaemonActionAuthorityToken) => daemonActionAuthorityIsCurrent(
    input.authorityRef.current,
    input.lifecycleRef.current,
    token,
  ) && input.isCapabilityAuthorized()

  const observeAuthority = () => {
    const continuation = continuationRef.current
    if (!continuation || authorityIsCurrent(authorityTokenFor(continuation))) return
    runIdRef.current += 1
    setDetailLoading(false)
    setDetailInterrupted(true)
    setDetailError("服务权限已撤销，已保留已读取内容；状态恢复后请显式续读")
  }

  const handleFailure = (error: unknown, continuation: DetailContinuation, runId: number) => {
    if (runId !== runIdRef.current) return
    const failure = bridgeFailureKind(error)
    setDetailLoading(false)
    if (failure === "stale_selection" || failure === "selection_missing") {
      continuationRef.current = null
      setDetail(null)
      setFullText("")
      setBodyComplete(false)
      setDetailInterrupted(false)
      input.onStaleSelection()
      setDetailError(failure === "stale_selection"
        ? "该简历已切换到新版本，私密详情已清除；请刷新搜索"
        : "该简历版本已删除或不再发布，私密详情已清除；请刷新搜索")
      return
    }
    if (failure === "unavailable" || failure === "overload" || !authorityIsCurrent(authorityTokenFor(continuation))) {
      continuationRef.current = continuation
      setDetailInterrupted(true)
      setDetailError(failure === "overload" ? "详情读取繁忙，已停止分页；可显式续读" : "详情读取已中断，已保留当前内容；daemon 就绪后可显式续读")
      return
    }
    continuationRef.current = null
    setDetailInterrupted(false)
    if (!continuation.metadataLoaded) {
      setDetail(null)
      setFullText("")
      setBodyComplete(false)
    }
    setDetailError("简历详情读取失败；该请求不会自动重放")
  }

  const hydratePages = async (continuation: DetailContinuation) => {
    const authority = input.isCapabilityAuthorized()
      ? captureDaemonActionAuthority(input.authorityRef.current, input.lifecycleRef.current)
      : null
    if (!authority) {
      observeAuthority()
      return
    }
    const runId = runIdRef.current + 1
    runIdRef.current = runId
    continuation.authorityEpoch = authority.epoch
    continuation.generation = authority.generation
    continuationRef.current = continuation
    setDetailLoading(true)
    setDetailInterrupted(false)
    setDetailError("")
    try {
      for (let page = continuation.pagesRead; page < MAX_DETAIL_PAGES; page += 1) {
        if (!authorityIsCurrent(authority)) {
          throw { code: "daemon_unavailable", message: "daemon action authority changed" }
        }
        const requestedOffset = continuation.nextOffset
        const requestId = `gui-hydrate-${crypto.randomUUID()}`
        const hydrated = await hydrateDetail(requestId, continuation.hit.selection, requestedOffset)
        if (runId !== runIdRef.current) return
        if (!authorityIsCurrent(authority)) {
          throw { code: "daemon_unavailable", message: "daemon action authority changed" }
        }
        if (hydrated.body.schema_version === "resume-ir.error.v2") {
          throw { code: hydrated.body.error.code, message: "detail service unavailable" }
        }
        if (hydrated.http_status !== 200 || hydrated.body.schema_version !== "resume-ir.detail-hydrate-response.v3") throw new Error("hydrate unavailable")
        if (hydrated.body.request_id !== requestId || !sameSearchSelection(hydrated.body.selection, continuation.hit.selection)) {
          throw { code: "bridge_protocol_error", message: "response context mismatch" }
        }
        const bodyPage = hydrated.body.document.body_page
        if (bodyPage.offset_bytes !== requestedOffset) throw new Error("hydrate cursor mismatch")
        continuation.text += bodyPage.text
        continuation.pagesRead += 1
        continuation.nextOffset = bodyPage.next_offset_bytes
        setFullText(continuation.text)
        if (bodyPage.complete) {
          continuationRef.current = null
          setBodyComplete(true)
          break
        }
        if (bodyPage.next_offset_bytes <= requestedOffset) throw new Error("hydrate cursor stalled")
      }
      if (continuation.pagesRead >= MAX_DETAIL_PAGES) continuationRef.current = null
    } catch (error) {
      handleFailure(error, continuation, runId)
    } finally {
      if (runId === runIdRef.current) setDetailLoading(false)
    }
  }

  const open = async (hit: SearchHit) => {
    const authority = input.isCapabilityAuthorized()
      ? captureDaemonActionAuthority(input.authorityRef.current, input.lifecycleRef.current)
      : null
    if (!authority || input.service === "repairing") return
    const runId = runIdRef.current + 1
    runIdRef.current = runId
    const continuation: DetailContinuation = { hit, authorityEpoch: authority.epoch, generation: authority.generation, nextOffset: 0, text: "", pagesRead: 0, metadataLoaded: false }
    continuationRef.current = continuation
    setDetail(null); setDetailLoading(true); setDetailInterrupted(false); setDetailError(""); setFullText(""); setBodyComplete(false)
    if (input.preview) {
      const candidateName = hit.file_name.replace(/\.[^.]+$/, "").replaceAll("_", " ").split(" ")[0]
      setDetail({ file_name: hit.file_name, source_byte_size: 128420, parse_version: "parser-preview", schema_version: "schema-v27", language_set: ["zh"], page_count: 2, quality_score: 0.96, fields_truncated: false, fields: [{ type: "name", value: candidateName, confidence: 0.98 }, { type: "skill", value: "Java", confidence: 0.96 }, { type: "skill", value: "Kafka", confidence: 0.94 }, { type: "location", value: "上海", confidence: 0.91 }], snippet: hit.snippet })
      continuationRef.current = null; setFullText("高级后端工程师\n\n核心技能：Java、Kafka、分布式系统、支付清结算。\n\n工作经历\n负责高吞吐消息管道与交易系统稳定性建设，持续优化延迟和资源使用。\n\n项目经历\n主导支付网关重构及异步对账平台建设。"); setBodyComplete(true); setDetailLoading(false); return
    }
    try {
      const requestId = `gui-detail-${crypto.randomUUID()}`
      const reply = await readDetail(requestId, hit.selection)
      if (runId !== runIdRef.current) return
      if (!authorityIsCurrent(authority)) throw { code: "daemon_unavailable", message: "daemon action authority changed" }
      if (reply.body.schema_version === "resume-ir.error.v2") {
        throw { code: reply.body.error.code, message: "detail service unavailable" }
      }
      if (reply.http_status !== 200 || reply.body.schema_version !== "resume-ir.detail-response.v3") throw new Error("detail unavailable")
      if (reply.body.request_id !== requestId || !sameSearchSelection(reply.body.selection, hit.selection)) throw { code: "bridge_protocol_error", message: "response context mismatch" }
      setDetail({ ...reply.body.document, file_name: hit.file_name })
      continuation.metadataLoaded = true
      await hydratePages(continuation)
    } catch (error) {
      handleFailure(error, continuation, runId)
    }
  }

  const resume = async () => {
    const continuation = continuationRef.current
    if (!continuation || !input.isCapabilityAuthorized() || !captureDaemonActionAuthority(input.authorityRef.current, input.lifecycleRef.current) || input.service === "repairing") return
    if (!continuation.metadataLoaded) await open(continuation.hit)
    else await hydratePages(continuation)
  }

  return {
    detail,
    detailLoading,
    detailError,
    fullText,
    bodyComplete,
    detailInterrupted,
    open,
    resume,
    reset,
    observeAuthority,
    observeLifecycle,
  }
}
