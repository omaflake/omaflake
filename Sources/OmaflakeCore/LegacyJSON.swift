// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Files written before the rename flavor → recipe (plan B92) carry the key "flavor" in instance.json and image.json.
/// They are read as if it were "recipe"; the next save writes the new key.
enum LegacyJSON {
    static func recipeKey(_ data: Data) -> Data {
        guard var object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              object["recipe"] == nil, let old = object.removeValue(forKey: "flavor") else { return data }
        object["recipe"] = old
        return (try? JSONSerialization.data(withJSONObject: object)) ?? data
    }
}
