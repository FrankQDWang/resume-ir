#!/usr/bin/env -S uv run --python 3.13 --with onnx==1.19.0 --with 'onnxruntime[quantization]==1.27.0' --with tokenizers==0.22.2
"""Run the bounded Issue #295 public-synthetic embedding artifact matrix."""

from __future__ import annotations

import argparse, hashlib, importlib.util, json, math, os, random, shutil, statistics, subprocess, sys, tempfile, time, urllib.request, unittest
from pathlib import Path
from typing import Any

import numpy as np
import onnx
from onnxruntime.quantization import CalibrationDataReader, CalibrationMethod, QuantFormat, QuantType, quantize_static
from onnxruntime.quantization.shape_inference import quant_pre_process
from tokenizers import Tokenizer

ROOT = Path(__file__).resolve().parents[2]
SCHEMA, STREAM, DIMENSION = "resume-ir.embedding-artifact-matrix.v1", "resume-ir.embedding-stream.v1", 384
REVISION = "614241f622f53c4eeff9890bdc4f31cfecc418b3"
FP32_SHA, FP32_BYTES = "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665", 470_268_510
FP32_URL = f"https://huggingface.co/intfloat/multilingual-e5-small/resolve/{REVISION}/onnx/model.onnx"
BUCKETS, PRIMARY, BLOCKS, SEED = (8, 32, 96, 256, 512), 512, 8, 295
MAX_REPORT = 64 * 1024
VARIANTS = {
    "current_dynamic_u8s8": "intfloat-multilingual-e5-small-qint8-r1",
    "fp32": "intfloat-multilingual-e5-small-fp32-exp-r1",
    "static_qdq_s8s8": "intfloat-multilingual-e5-small-static-qdq-s8s8-exp-r1",
    "static_qoperator_u8s8": "intfloat-multilingual-e5-small-static-qoperator-u8s8-exp-r1",
}
PRIVACY = {key: False for key in ("contains_raw_resume_text", "contains_raw_query", "contains_candidate_results", "contains_local_paths", "contains_vectors", "contains_token_content", "contains_model_bytes", "contains_pids", "contains_raw_profiler_data")}

def load_helper(name: str, file: str) -> Any:
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts/local" / file)
    if spec is None or spec.loader is None: raise RuntimeError("helper_unavailable")
    module = importlib.util.module_from_spec(spec); sys.modules[name] = module; spec.loader.exec_module(module); return module

PRE = load_helper("prepacking_witness", "embedding-prepacking-benchmark.py")
PROF = load_helper("operator_profile", "embedding-onnx-operator-profile.py")

def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""): digest.update(chunk)
    return digest.hexdigest()

def owner_dir(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True); path.chmod(0o700)

def exact_text(tokenizer: Tokenizer, role: str, target: int) -> str:
    low, high = 1, 1024
    while low <= high:
        count = (low + high) // 2; text = " ".join(["test"] * count)
        length = len(tokenizer.encode(f"{role}: {text}", add_special_tokens=True).ids)
        if length == target: return text
        if length < target: low = count + 1
        else: high = count - 1
    raise RuntimeError("calibration_token_count_unavailable")

def calibration_batches(runtime: Path) -> list[dict[str, np.ndarray]]:
    tokenizer = Tokenizer.from_file(str(runtime / "tokenizer.json")); tokenizer.enable_truncation(max_length=512)
    batches = []
    for target in BUCKETS:
        inputs = [(role, exact_text(tokenizer, role, target)) for role in ("query", "passage") for _ in range(10)]
        for offset in range(0, 20, 4):
            encoded = [tokenizer.encode(f"{role}: {text}", add_special_tokens=True) for role, text in inputs[offset:offset + 4]]
            width = max(len(item.ids) for item in encoded)
            def values(field: str) -> np.ndarray:
                rows = [getattr(item, field) + [0] * (width - len(getattr(item, field))) for item in encoded]
                return np.asarray(rows, dtype=np.int64)
            batches.append({"input_ids": values("ids"), "attention_mask": values("attention_mask"), "token_type_ids": values("type_ids")})
    return batches

class Reader(CalibrationDataReader):
    def __init__(self, batches: list[dict[str, np.ndarray]]): self.iterator = iter(batches)
    def get_next(self) -> dict[str, np.ndarray] | None: return next(self.iterator, None)

def manifest(root: Path, variant: str, model: Path, quant: dict[str, Any]) -> Path:
    value = {"schema_version":"resume-ir.embedding-artifact-experiment.v1","variant":variant,"model_id":VARIANTS[variant],"upstream_model_id":"intfloat/multilingual-e5-small","upstream_revision":REVISION,"dimension":DIMENSION,"generator_onnx_version":"1.19.0","generator_ort_version":"1.27.0","quantization":quant,"model":{"file":"model.onnx","bytes":model.stat().st_size,"sha256":sha256(model)}}
    path = root / "experiment.json"; path.write_text(json.dumps(value, separators=(",", ":"))); path.chmod(0o600); return path

def quant_identity(method: str, format_: str, activation: str, calibration: bool) -> dict[str, Any]:
    return {"method":method,"format":format_,"activation_type":activation,"weight_type":"float32" if method == "none" else "qint8","calibration_id":"public_synthetic_100_minmax_matmul_v1" if calibration else None,"op_types":["MatMul"],"graph_optimization":"disabled"}

def generate(root: Path, runtime: Path) -> dict[str, Path]:
    download = root / "upstream-fp32.onnx"
    urllib.request.urlretrieve(FP32_URL, download)
    if download.stat().st_size != FP32_BYTES or sha256(download) != FP32_SHA: raise RuntimeError("fp32_identity_mismatch")
    roots = {name: root / name for name in VARIANTS if name != "current_dynamic_u8s8"}
    for path in roots.values(): owner_dir(path)
    fp32 = roots["fp32"] / "model.onnx"; shutil.copyfile(download, fp32)
    result = {"fp32": manifest(roots["fp32"], "fp32", fp32, quant_identity("none", "fp32", "float32", False))}
    prepared = root / "prepared.onnx"; quant_pre_process(str(download), str(prepared), skip_optimization=True, skip_symbolic_shape=True)
    batches = calibration_batches(runtime)
    specs = [("static_qdq_s8s8", QuantFormat.QDQ, QuantType.QInt8, "qdq", "qint8"),("static_qoperator_u8s8", QuantFormat.QOperator, QuantType.QUInt8, "qoperator", "quint8")]
    for name, format_enum, activation_enum, format_name, activation_name in specs:
        model = roots[name] / "model.onnx"
        quantize_static(str(prepared), str(model), Reader(batches), quant_format=format_enum, activation_type=activation_enum, weight_type=QuantType.QInt8, op_types_to_quantize=["MatMul"], per_channel=False, calibrate_method=CalibrationMethod.MinMax, extra_options={"DisableShapeInference": True})
        ops = {node.op_type for node in onnx.load(model, load_external_data=False).graph.node}
        if "DynamicQuantizeLinear" in ops or (format_name == "qdq" and not {"QuantizeLinear","DequantizeLinear"} <= ops) or (format_name == "qoperator" and not ({"QLinearMatMul","MatMulInteger"} & ops)): raise RuntimeError("candidate_graph_invalid")
        result[name] = manifest(roots[name], name, model, quant_identity("static", format_name, activation_name, True))
    return result

class Resident:
    def __init__(self, binary: Path, runtime: Path, variant: str, candidate: Path | None = None, profile: Path | None = None, threads: int = 3):
        env = os.environ.copy(); env.update({"RESUME_IR_EMBEDDING_RUNTIME_DIR":str(runtime),"RESUME_IR_EMBEDDING_MODEL_ID":VARIANTS[variant],"RESUME_IR_EMBEDDING_DIMENSION":str(DIMENSION),"RESUME_IR_EMBEDDING_INTRA_THREADS":str(threads)})
        if candidate: env["RESUME_IR_EMBEDDING_ARTIFACT_EXPERIMENT_MANIFEST"] = str(candidate)
        if profile: env["RESUME_IR_EMBEDDING_PROFILE_OUTPUT_PREFIX"] = str(profile)
        mode = "--resident" if candidate is None else "--resident-artifact-matrix"
        if profile: mode = "--resident-artifact-profile"
        # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit -- shell=False with a resolved, explicit local binary.
        started=time.perf_counter_ns(); self.process = subprocess.Popen([str(binary), mode], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL); self.request_id = 0; self.model_id=VARIANTS[variant]
        if self.process.stdin is None or self.process.stdout is None or PROF.read_frame(self.process.stdout, 60) != {"type":"ready","schema_version":STREAM,"model_id":VARIANTS[variant],"dimension":DIMENSION}: self.stop(); raise RuntimeError("resident_start_failed")
        self.ready_ms=(time.perf_counter_ns()-started)/1_000_000
    def request(self, inputs: list[dict[str,str]]) -> tuple[int, int, tuple[tuple[float,...],...], int, int]:
        self.request_id += 1; started = time.perf_counter_ns(); PROF.write_frame(self.process.stdin,{"schema_version":STREAM,"request_id":self.request_id,"model_id":self.model_id,"dimension":DIMENSION,"inputs":inputs})
        response = PROF.read_frame(self.process.stdout, 60); wall = (time.perf_counter_ns()-started)//1000; vectors, telemetry = response.get("vectors"), response.get("telemetry")
        if response.get("type") != "result" or not isinstance(vectors,list) or len(vectors) != len(inputs) or not isinstance(telemetry,dict): raise RuntimeError("resident_result_invalid")
        checked = tuple(tuple(float(value) for value in vector) for vector in vectors)
        if any(len(vector)!=DIMENSION or any(not math.isfinite(value) for value in vector) or abs(math.sqrt(sum(value*value for value in vector))-1)>1e-4 for vector in checked): raise RuntimeError("vector_invalid")
        active,padded=int(telemetry["active_token_count"]),int(telemetry["padded_token_count"])
        if padded < active: raise RuntimeError("token_accounting_invalid")
        return active,padded,checked,int(telemetry["onnx_us"]),wall
    def close(self) -> None:
        self.process.stdin.close(); status=self.process.wait(timeout=20)
        if status != 0: raise RuntimeError("resident_exit_failed")
        self.process=None
    def stop(self) -> None:
        if getattr(self,"process",None) is not None and self.process.poll() is None: self.process.terminate(); self.process.wait(timeout=5)

def start(binary: Path, runtime: Path, variant: str, manifest_path: Path | None = None, profile: Path | None = None, threads: int = 3) -> Resident:
    return Resident(binary,runtime,variant,manifest_path,profile,threads)

def latin_square() -> list[list[str]]:
    names=list(VARIANTS); base=[[0,1,3,2],[1,2,0,3],[2,3,1,0],[3,0,2,1]]; return [[names[index] for index in row] for row in base+list(reversed(base))]

def run_session(binary: Path, runtime: Path, variant: str, candidate: Path | None, workloads: dict[int,list[dict[str,str]]], block: int) -> dict[int,tuple[float,float]]:
    resident=start(binary,runtime,variant,candidate); samples={bucket:([],[]) for bucket in BUCKETS}
    try:
        deadline=time.monotonic()+30
        while time.monotonic()<deadline: resident.request(workloads[PRIMARY]); time.sleep(min(1,max(0,deadline-time.monotonic())))
        order=list(BUCKETS); random.Random(SEED+block).shuffle(order)
        for bucket in order:
            for _ in range(20 if bucket==PRIMARY else 10):
                active,_,_,onnx_us,wall_us=resident.request(workloads[bucket])
                if active != 4*bucket: raise RuntimeError("token_accounting_invalid")
                samples[bucket][0].append(onnx_us); samples[bucket][1].append(wall_us)
        resident.close(); return {bucket:(statistics.median(values[0]),statistics.median(values[1])) for bucket,values in samples.items()}
    finally: resident.stop()

def quality(binary: Path, runtime: Path, variant: str, candidate: Path | None) -> Any:
    resident=start(binary,runtime,variant,candidate); vectors=[]; signatures=[]
    try:
        inputs=PRE.quality_workload()
        for offset in range(0,len(inputs),4):
            active,padded,current,_,_=resident.request(inputs[offset:offset+4]); vectors.extend(current); signatures.append((len(current),active,padded))
        resident.close(); return PRE.QualityResult(tuple(vectors),BUCKETS,tuple(signatures))
    finally: resident.stop()

def interval(control: list[float], candidate: list[float]) -> tuple[float,float]: return PRE.bootstrap_improvement_interval(control,candidate,SEED)
def improvement(control: float, candidate: float) -> float: return (control-candidate)*100/control

def resources(binary: Path, runtime: Path, variant: str, candidate: Path | None, inputs: list[dict[str,str]]) -> dict[str,float]:
    result={}
    for label,threads in (("h0",1),("h2",3)):
        resident=start(binary,runtime,variant,candidate,threads=threads)
        try:
            resident.request(inputs); result[f"{label}_ready_ms"]=resident.ready_ms; result[f"{label}_rss_bytes"]=PRE.rss_bytes(resident.process.pid); result[f"{label}_physical_footprint_bytes"]=PRE.physical_footprint(resident.process.pid)[1]; resident.close()
        finally: resident.stop()
    return result

def decide(blocks: dict[str,list[dict[int,tuple[float,float]]]], qualities: dict[str,dict[str,Any]], resource: dict[str,dict[str,float]]) -> dict[str,Any]:
    control=blocks["current_dynamic_u8s8"]; passing=[]; gates={}
    for name in list(VARIANTS)[1:]:
        primary=[item[PRIMARY] for item in blocks[name]]; base=[item[PRIMARY] for item in control]
        onnx_ci=interval([x[0] for x in base],[x[0] for x in primary]); wall_ci=interval([x[1] for x in base],[x[1] for x in primary]); gain=improvement(statistics.median(x[0] for x in base),statistics.median(x[0] for x in primary))
        sensitivity=max(-improvement(statistics.median(x[b][0] for x in control),statistics.median(x[b][0] for x in blocks[name])) for b in BUCKETS[:-1])
        ready_delta=resource[name]["h2_ready_ms"]-resource["current_dynamic_u8s8"]["h2_ready_ms"]; ready_ok=ready_delta<=1000 and ready_delta*100/resource["current_dynamic_u8s8"]["h2_ready_ms"]<=10
        resource_ok=resource[name]["h0_physical_footprint_bytes"]<=512*1024**2 and resource[name]["h2_physical_footprint_bytes"]<=1536*1024**2
        accepted=gain>=10 and onnx_ci[0]>0 and wall_ci[0]>0 and sensitivity<=3 and qualities[name]["passed"] and ready_ok and resource_ok
        gates[name]={"primary_onnx_improvement_pct":gain,"onnx_bootstrap_95pct":onnx_ci,"wall_bootstrap_95pct":wall_ci,"maximum_sensitivity_regression_pct":sensitivity,"quality_pass":qualities[name]["passed"],"ready_pass":ready_ok,"resource_pass":resource_ok,"accepted":accepted}
        if accepted: passing.append(name)
    passing.sort(key=lambda name: statistics.median(x[PRIMARY][0] for x in blocks[name]))
    if not passing: return {"outcome":"lost","winner":None,"gates":gates}
    if len(passing)>1:
        first,second=passing[:2]; ci=interval([x[PRIMARY][0] for x in blocks[second]],[x[PRIMARY][0] for x in blocks[first]])
        if ci[0]<=0: return {"outcome":"inconclusive","winner":None,"gates":gates,"tie_interval":ci}
    return {"outcome":"won","winner":passing[0],"gates":gates}

def profile_winner(binary: Path, runtime: Path, winner: str, candidate: Path, inputs: list[dict[str,str]]) -> dict[str,Any]:
    plain=start(binary,runtime,winner,candidate)
    try: _,_,control,_,_=plain.request(inputs); plain.close()
    finally: plain.stop()
    captures=[]
    with tempfile.TemporaryDirectory(prefix="resume-ir-artifact-profile-") as raw:
        root=Path(raw); root.chmod(0o700)
        for index in range(5):
            prefix=root/f"capture-{index}"; resident=start(binary,runtime,winner,candidate,prefix)
            try:
                deadline=time.monotonic()+30
                while time.monotonic()<deadline: resident.request(inputs); time.sleep(min(1,max(0,deadline-time.monotonic())))
                current=None
                for _ in range(20): _,_,current,_,_=resident.request(inputs)
                resident.close(); traces=list(root.glob(f"capture-{index}*.json"))
                if len(traces)!=1 or current!=control: raise RuntimeError("winner_profile_control_failed")
                captures.append(PROF.read_trace(traces[0],20))
            finally: resident.stop()
        top=[capture["families"][0]["family"] for capture in captures]; family=max(set(top),key=top.count); stable=top.count(family)>=4
        dynamic=statistics.median(next((item["node_share"] for item in capture["families"] if item["family"]=="dynamic_quantization"),0) for capture in captures)
        resident=start(binary,runtime,winner,candidate); family_sample=None; method="xctrace_time_profiler"
        try:
            resident.request(inputs); trace=root/"time-profile.trace"; sampler=subprocess.Popen(["xcrun","xctrace","record","--template","Time Profiler","--attach",str(resident.process.pid),"--time-limit","20s","--output",str(trace),"--no-prompt"],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); PROF.drive_sampler(resident,inputs,sampler,35)
            if sampler.returncode==0:
                exported=subprocess.run(["xcrun","xctrace","export","--input",str(trace),"--xpath",'/trace-toc/run[@number="1"]/data/table'],capture_output=True,timeout=30,check=False)
                if exported.returncode==0: family_sample=PROF.symbol_family(exported.stdout.decode(errors="ignore"))
            if family_sample is None:
                method="sample_fallback"; sample=root/"sample.txt"; fallback=subprocess.Popen(["sample",str(resident.process.pid),"20","1","-file",str(sample)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); PROF.drive_sampler(resident,inputs,fallback,35)
                if fallback.returncode==0 and sample.exists(): family_sample=PROF.symbol_family(sample.read_text(errors="ignore"))
            resident.close()
        finally: resident.stop()
    conflicts=family_sample is None or (family_sample in {"allocator","scheduling"} and family_sample!=family)
    return {"captures":5,"exact_vector_control":True,"top_family":family,"top_family_captures":top.count(family),"dynamic_quantization_share_median":dynamic,"dynamic_share_reduced":dynamic<=0.4635,"cross_check":{"method":method,"symbol_family":family_sample or "unavailable","conflicts":conflicts},"passed":stable and dynamic<=0.4635 and not conflicts}

class Tests(unittest.TestCase):
    def test_latin_square_balances_positions(self):
        rows=latin_square(); self.assertEqual(len(rows),BLOCKS)
        for position in range(4): self.assertEqual({name:sum(row[position]==name for row in rows) for name in VARIANTS},{name:2 for name in VARIANTS})
    def test_decision_lost_and_unique(self):
        control=[{b:(100.0,110.0) for b in BUCKETS} for _ in range(8)]; blocks={name:[dict(row) for row in control] for name in VARIANTS}; qualities={name:{"passed":True} for name in VARIANTS}; resource={name:{"h0_physical_footprint_bytes":1,"h2_physical_footprint_bytes":1,"h2_ready_ms":100} for name in VARIANTS}
        self.assertEqual(decide(blocks,qualities,resource)["outcome"],"lost")
        blocks["fp32"]=[{b:(80.0,90.0) for b in BUCKETS} for _ in range(8)]; self.assertEqual(decide(blocks,qualities,resource)["winner"],"fp32")
        blocks["static_qdq_s8s8"]=[dict(row) for row in blocks["fp32"]]; self.assertEqual(decide(blocks,qualities,resource)["outcome"],"inconclusive")
    def test_report_boundary_rejects_non_finite_and_paths(self):
        with self.assertRaises(ValueError): encode({"x":float("nan")},("/private/root",))
        with self.assertRaises(ValueError): encode({"x":"/private/root"},("/private/root",))

def encode(report: dict[str,Any], denied: tuple[str,...]) -> bytes:
    raw=json.dumps(report,separators=(",",":"),allow_nan=False).encode()
    if len(raw)>MAX_REPORT or any(value and value.encode() in raw for value in denied) or any(marker in raw for marker in (b"/Users/",b"/private/",b"alpha",b"vector=")): raise ValueError("report_boundary_failed")
    return raw

def main() -> int:
    parser=argparse.ArgumentParser(description=__doc__); parser.add_argument("--self-test",action="store_true"); parser.add_argument("--binary",type=Path); parser.add_argument("--runtime-dir",type=Path); parser.add_argument("--out",type=Path); args=parser.parse_args()
    if args.self_test:
        suite=unittest.TestSuite(unittest.defaultTestLoader.loadTestsFromTestCase(case) for case in (Tests,PRE.SelfTests,PROF.SelfTests)); return 0 if unittest.TextTestRunner().run(suite).wasSuccessful() else 1
    if not all((args.binary,args.runtime_dir,args.out)): parser.error("binary, runtime-dir, and out are required")
    binary,runtime,out=args.binary.resolve(strict=True),args.runtime_dir.resolve(strict=True),args.out.resolve(strict=False)
    with tempfile.TemporaryDirectory(prefix="resume-ir-artifact-matrix-") as raw:
        root=Path(raw); root.chmod(0o700); manifests=generate(root,runtime); workloads=PROF.calibrate_workloads(binary,runtime,60)
        pack=json.loads((runtime/"runtime-pack.json").read_text()); control_model=next(item for item in pack["files"] if item["role"]=="model")
        artifacts={"current_dynamic_u8s8":{"model_bytes":control_model["bytes"],"sha256":control_model["sha256"],"runtime_pack_bytes":sum(item["bytes"] for item in pack["files"])}}
        artifacts.update({name:{"model_bytes":value["bytes"],"sha256":value["sha256"]} for name,path in manifests.items() for value in (json.loads(path.read_text())["model"],)})
        blocks={name:[] for name in VARIANTS}
        for block,row in enumerate(latin_square()):
            for name in row: blocks[name].append(run_session(binary,runtime,name,manifests.get(name),workloads,block))
        controls=quality(binary,runtime,"current_dynamic_u8s8",None); qualities={"current_dynamic_u8s8":{"passed":True}}
        for name in list(VARIANTS)[1:]: qualities[name]=PRE.quality_summary(controls,quality(binary,runtime,name,manifests[name]))
        resource={name:resources(binary,runtime,name,manifests.get(name),workloads[PRIMARY]) for name in VARIANTS}
        decision=decide(blocks,qualities,resource); profile=None
        if decision["outcome"]=="won":
            profile=profile_winner(binary,runtime,decision["winner"],manifests[decision["winner"]],workloads[PRIMARY])
            if not profile["passed"]: decision={**decision,"outcome":"inconclusive","winner":None,"profile_rejected_candidate":True}
        summaries={name:{str(bucket):{"onnx_us_median":statistics.median(row[bucket][0] for row in values),"wall_us_median":statistics.median(row[bucket][1] for row in values)} for bucket in BUCKETS} for name,values in blocks.items()}
        report={"schema_version":SCHEMA,"issue":295,"source":"public_synthetic","upstream_revision":REVISION,"workload":{"blocks":BLOCKS,"sessions":32,"batch_size":4,"warmup_seconds":30,"primary_tokens":PRIMARY,"measured_primary":20,"measured_sensitivity":10,"intra_threads":3},"artifacts":artifacts,"variants":summaries,"quality":qualities,"resources":resource,"decision":decision,"winner_profile":profile,"privacy":PRIVACY}
        encoded=encode(report,(str(root),str(runtime),str(binary),str(out.parent))); out.parent.mkdir(parents=True,exist_ok=True); out.write_bytes(encoded+b"\n")
    return 0 if decision["outcome"] in {"won","lost","inconclusive"} else 2

if __name__ == "__main__": raise SystemExit(main())
