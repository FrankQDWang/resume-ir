@preconcurrency import CoreML
import Foundation

private let batch = 4
private let tokens = 512
private let dimension = 384

private struct Arguments {
    let model: URL
    let inputIDs: URL
    let attentionMask: URL
    let output: URL
    let warmups: Int
    let repetitions: Int
    let includeComputePlan: Bool
}

private enum RunnerError: Error {
    case invalidArguments
    case invalidTensor
    case invalidOutput
}

private func arguments() throws -> Arguments {
    let values = CommandLine.arguments
    guard (values.count == 7 || values.count == 8),
          let warmups = Int(values[5]), warmups > 0, warmups <= 32,
          let repetitions = Int(values[6]), repetitions > 0, repetitions <= 128,
          values.count == 7 || values[7] == "0" || values[7] == "1" else {
        throw RunnerError.invalidArguments
    }
    return Arguments(
        model: URL(fileURLWithPath: values[1]),
        inputIDs: URL(fileURLWithPath: values[2]),
        attentionMask: URL(fileURLWithPath: values[3]),
        output: URL(fileURLWithPath: values[4]),
        warmups: warmups,
        repetitions: repetitions,
        includeComputePlan: values.count == 7 || values[7] == "1"
    )
}

private func multiArray(at url: URL) throws -> MLMultiArray {
    let data = try Data(contentsOf: url, options: .mappedIfSafe)
    let count = batch * tokens
    guard data.count == count * MemoryLayout<Int32>.size else {
        throw RunnerError.invalidTensor
    }
    let array = try MLMultiArray(shape: [batch, tokens] as [NSNumber], dataType: .int32)
    try array.withUnsafeMutableBytes { destination, _ in
        guard destination.count == data.count else { throw RunnerError.invalidTensor }
        data.copyBytes(to: destination)
    }
    return array
}

private func deviceName(_ device: MLComputeDevice) -> String {
    switch device {
    case .cpu: return "cpu"
    case .gpu: return "gpu"
    case .neuralEngine: return "neural_engine"
    @unknown default: return "unknown"
    }
}

private func accumulate(
    block: MLModelStructure.Program.Block,
    plan: MLComputePlan,
    costs: inout [String: Double],
    operations: inout Int
) {
    for operation in block.operations {
        if let usage = plan.deviceUsage(for: operation),
           let cost = plan.estimatedCost(of: operation) {
            costs[deviceName(usage.preferred), default: 0.0] += cost.weight
        }
        operations += 1
        for nested in operation.blocks {
            accumulate(block: nested, plan: plan, costs: &costs, operations: &operations)
        }
    }
}

@main
private enum Main {
    @MainActor
    static func main() async throws {
        let args = try arguments()
        let configuration = MLModelConfiguration()
        configuration.computeUnits = .all
        var hints = MLOptimizationHints()
        hints.specializationStrategy = .fastPrediction
        configuration.optimizationHints = hints

        let loadStarted = ContinuousClock.now
        let model = try MLModel(contentsOf: args.model, configuration: configuration)
        let loadMs = Double(loadStarted.duration(to: ContinuousClock.now).components.attoseconds) / 1e15
        let provider = try MLDictionaryFeatureProvider(dictionary: [
            "input_ids": MLFeatureValue(multiArray: try multiArray(at: args.inputIDs)),
            "attention_mask": MLFeatureValue(multiArray: try multiArray(at: args.attentionMask)),
        ])
        for _ in 0..<args.warmups {
            _ = try await model.prediction(from: provider)
        }

        var samples = [Double]()
        var lastOutput: MLMultiArray?
        for _ in 0..<args.repetitions {
            let started = ContinuousClock.now
            let prediction = try await model.prediction(from: provider)
            let elapsed = started.duration(to: ContinuousClock.now)
            samples.append(Double(elapsed.components.attoseconds) / 1e15)
            lastOutput = prediction.featureValue(for: "embeddings")?.multiArrayValue
        }
        guard let embeddings = lastOutput, embeddings.count == batch * dimension else {
            throw RunnerError.invalidOutput
        }
        var output = Data(count: embeddings.count * MemoryLayout<Float>.size)
        try output.withUnsafeMutableBytes { raw in
            guard let destination = raw.bindMemory(to: Float.self).baseAddress else {
                throw RunnerError.invalidOutput
            }
            for index in 0..<embeddings.count {
                destination[index] = embeddings[index].floatValue
            }
        }
        try output.write(to: args.output, options: .atomic)

        var costs = ["cpu": 0.0, "gpu": 0.0, "neural_engine": 0.0, "unknown": 0.0]
        var operationCount = 0
        if args.includeComputePlan {
            let plan = try await MLComputePlan.load(
                contentsOf: args.model,
                configuration: configuration
            )
            if case .program(let program) = plan.modelStructure {
                for function in program.functions.values {
                    accumulate(
                        block: function.block,
                        plan: plan,
                        costs: &costs,
                        operations: &operationCount
                    )
                }
            }
        }
        let payload: [String: Any] = [
            "schema_version": "resume-ir.coreml-b4x512-run.v1",
            "load_ms": loadMs,
            "samples_ms": samples,
            "operation_count": operationCount,
            "estimated_cost_by_preferred_device": costs,
            "vector_count": batch,
            "dimension": dimension,
            "contains_private_data": false,
        ]
        let encoded = try JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys])
        FileHandle.standardOutput.write(encoded)
        FileHandle.standardOutput.write(Data([0x0A]))
    }
}
