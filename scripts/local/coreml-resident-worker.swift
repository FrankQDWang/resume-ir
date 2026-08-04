@preconcurrency import CoreML
import Foundation

private let tokens = 512
private let dimension = 384
private let readyByte = Data([0xa5])

private enum WorkerError: Error {
    case invalidArguments
    case invalidInput
    case invalidOutput
}

private func readExact(_ handle: FileHandle, bytes: Int) throws -> Data? {
    var result = Data()
    while result.count < bytes {
        guard let chunk = try handle.read(upToCount: bytes - result.count), !chunk.isEmpty else {
            if result.isEmpty { return nil }
            throw WorkerError.invalidInput
        }
        result.append(chunk)
    }
    return result
}

private func copyTensor(_ data: Data, into array: MLMultiArray) throws {
    try array.withUnsafeMutableBytes { destination, _ in
        guard destination.count == data.count else { throw WorkerError.invalidInput }
        data.copyBytes(to: destination)
    }
}

private func clearTensor(_ array: MLMultiArray) throws {
    _ = array.withUnsafeMutableBytes { destination, _ in
        destination.initializeMemory(as: UInt8.self, repeating: 0)
    }
}

@main
private enum Main {
    @MainActor
    static func main() async throws {
        let arguments = CommandLine.arguments
        guard arguments.count == 3,
              let batch = Int(arguments[2]), batch == 1 || batch == 4 else {
            throw WorkerError.invalidArguments
        }
        let modelURL = URL(fileURLWithPath: arguments[1])
        let configuration = MLModelConfiguration()
        configuration.computeUnits = .all
        var hints = MLOptimizationHints()
        hints.specializationStrategy = .fastPrediction
        configuration.optimizationHints = hints
        let model = try MLModel(contentsOf: modelURL, configuration: configuration)
        let inputIDs = try MLMultiArray(shape: [batch, tokens] as [NSNumber], dataType: .int32)
        let attentionMask = try MLMultiArray(
            shape: [batch, tokens] as [NSNumber],
            dataType: .int32
        )
        try clearTensor(inputIDs)
        try clearTensor(attentionMask)
        let provider = try MLDictionaryFeatureProvider(dictionary: [
            "input_ids": MLFeatureValue(multiArray: inputIDs),
            "attention_mask": MLFeatureValue(multiArray: attentionMask),
        ])
        _ = try await model.prediction(from: provider)
        let inputBytes = batch * tokens * MemoryLayout<Int32>.size
        let input = FileHandle.standardInput
        let output = FileHandle.standardOutput
        try output.write(contentsOf: readyByte)

        while let ids = try readExact(input, bytes: inputBytes) {
            guard let mask = try readExact(input, bytes: inputBytes) else {
                throw WorkerError.invalidInput
            }
            try copyTensor(ids, into: inputIDs)
            try copyTensor(mask, into: attentionMask)
            let prediction = try await model.prediction(from: provider)
            guard let embeddings = prediction.featureValue(for: "embeddings")?.multiArrayValue,
                  embeddings.count == batch * dimension else {
                throw WorkerError.invalidOutput
            }
            var encoded = Data(count: embeddings.count * MemoryLayout<Float>.size)
            try encoded.withUnsafeMutableBytes { raw in
                guard let destination = raw.bindMemory(to: Float.self).baseAddress else {
                    throw WorkerError.invalidOutput
                }
                for index in 0..<embeddings.count {
                    let value = embeddings[index].floatValue
                    guard value.isFinite else { throw WorkerError.invalidOutput }
                    destination[index] = value
                }
            }
            try output.write(contentsOf: encoded)
        }
    }
}
