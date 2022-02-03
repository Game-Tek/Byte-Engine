#pragma once

template<typename T>
bool Contains(const T& a, GTSL::Range<const T*> params) {
	for (const auto& e : params) { if (e == a) { return true; } }
	return false;
}

struct PermutationManager : Object {
	struct ShaderGenerationData {
		GTSL::StaticVector<GPipeline::ElementHandle, 16> Scopes;
		GTSL::StaticVector<PermutationManager*, 16> Hierarchy;
	};

	PermutationManager(const GTSL::StringView instance_name, const GTSL::StringView class_name) : InstanceName(instance_name), ClassName(class_name) {

	}

	static void InitializePermutations(PermutationManager* start, GPipeline* pipeline) {
		ShaderGenerationData shader_generation_data;

		auto call = [&](PermutationManager* parent, auto&& self) -> void {
			parent->Initialize(pipeline, shader_generation_data);

			shader_generation_data.Hierarchy.EmplaceBack(parent);

			for (const auto& e : parent->Children) {
				self(e, self);
			}

			shader_generation_data.Hierarchy.PopBack();
		};

		call(start, call);
	}

	virtual void Initialize(GPipeline* pipeline, ShaderGenerationData& shader_generation_data) = 0;

	struct Result {
		GAL::ShaderType TargetSemantics;
		GTSL::StaticVector<GPipeline::ElementHandle, 16> Scopes;
		GTSL::StaticVector<GTSL::ShortString<16>, 4> Tags;
	};
	virtual void ProcessShader(GPipeline* pipeline, GTSL::JSONMember shaderGroupJson, GTSL::JSONMember shaderJson, GTSL::StaticVector<PermutationManager*, 16> hierarchy, GTSL::StaticVector<Result, 8>& batches) = 0;

	template<class A>
	PermutationManager* CreateChild(const GTSL::StringView name) {
		return Children.EmplaceBack(GTSL::SmartPointer<A, BE::TAR>(GetTransientAllocator(), name));
	}

	GTSL::StaticVector<GTSL::SmartPointer<PermutationManager, BE::TAR>, 8> Children;
	GTSL::StaticString<64> InstanceName;
	const GTSL::StaticString<64> ClassName;

	template<typename T>
	static T* Find(const GTSL::StringView class_name, const GTSL::Range<PermutationManager**> hierarchy) {
		for (auto& e : hierarchy) {
			if (e->ClassName == class_name) { //pseudo dynamic cast
				return static_cast<T*>(e);
			}
		}

		return nullptr;
	}

	GTSL::Range<const GTSL::ShortString<16>*> GetTagList() { return tags; }

	static auto ProcessShaders(PermutationManager* start, GPipeline* pipeline, GTSL::JSONMember shader_group_json, GTSL::JSONMember shader_json) {
		GTSL::StaticVector<Result, 8> batches;
		GTSL::StaticVector<PermutationManager*, 16> scopes;

		//there's an unintended side effect in this code, where the first permutation is called regardless of whether it can process the current domain or not
		auto call = [&](PermutationManager* parent, auto&& self) -> void {
			parent->ProcessShader(pipeline, shader_group_json, shader_json, scopes, batches);

			scopes.EmplaceBack(parent);

			for(const auto& e : parent->Children) {
				if (Contains(GTSL::ShortString<32>(shader_group_json[u8"domain"]), static_cast<const PermutationManager*>(e.GetData())->supportedDomains.GetRange())) {
					self(e, self);
				}
			}

			scopes.PopBack();
		};

		call(start, call);

		return batches;
	}

protected:
	void AddTag(const GTSL::StringView tag_string) { tags.EmplaceBack(tag_string); }
	void AddSupportedDomain(const GTSL::StringView domain_name) { supportedDomains.EmplaceBack(domain_name); }

private:
	GTSL::StaticVector<GTSL::ShortString<16>, 4> tags;
	GTSL::StaticVector<GTSL::ShortString<32>, 8> supportedDomains;
};