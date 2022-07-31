#pragma once

template<typename T>
bool Contains(const T& a, GTSL::Range<const T*> params) {
	for (const auto& e : params) { if (e == a) { return true; } }
	return false;
}

struct PermutationManager : Object {
	struct ShaderGenerationData {
		GTSL::StaticVector<GPipeline::ElementHandle, 16> Scopes;
		GTSL::StaticVector<const PermutationManager*, 16> Hierarchy;
	};

	using ShaderTag = GTSL::Pair<GTSL::ShortString<32>, GTSL::ShortString<32>>;

	PermutationManager(const GTSL::StringView instance_name, const GTSL::StringView class_name) : InstanceName(instance_name), ClassName(class_name) {

	}

	static GTSL::StaticString<8192> MakeShaderString(GTSL::StringView path, GTSL::StringView user_shader_code = {}) {
		GTSL::File file(ResourceManager::GetUserResourcePath(path, u8"txt"));

		GTSL::StaticBuffer<8192> buffer;

		GTSL::StaticString<8192> shaderCode;

		file.Read(buffer);

		uint32 i = 0;

		while(true) {
			auto s = i;

			while(i != buffer.GetLength() && *(buffer.begin() + i) != u8'@') {
				++i;
			}

			shaderCode += { i - s, i - s, reinterpret_cast<const utf8*>(buffer.begin()) + s };

			if(i == buffer.GetLength()) {
				break;
			}

			shaderCode += user_shader_code;

			i += 3;
		}

		return shaderCode;
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

	struct ShaderPermutation {
		GAL::ShaderType TargetSemantics;
		GTSL::StaticVector<GPipeline::ElementHandle, 8> Scopes;
		GTSL::StaticVector<ShaderTag, 4> Tags;
	};
	virtual void ProcessShader(GPipeline* pipeline, GTSL::JSONMember shaderGroupJson, GTSL::JSONMember shaderJson, const GTSL::Range<const PermutationManager**> hierarchy, GTSL::StaticVector<ShaderPermutation, 8>& batches) = 0;

	struct ShaderGroupDescriptor {
		GTSL::StaticString<1024> ShaderGroupJSON;
		GTSL::StaticVector<GTSL::StaticVector<ShaderPermutation, 4>, 4> Shaders;
	};
	virtual GTSL::Vector<ShaderGroupDescriptor, BE::TAR > MakeShaderGroups(GPipeline* pipeline, GTSL::Range<const PermutationManager**> hierarchy) = 0;

	static auto GetDefaultShaderGroups(PermutationManager* permutation_manager, GPipeline* pipeline) {
		GTSL::StaticVector<ShaderGroupDescriptor, 8> result1s;
		GTSL::StaticVector<const PermutationManager*, 16> hierarchy;

		auto call = [&](PermutationManager* parent, auto&& self) -> void {
			auto res = parent->MakeShaderGroups(pipeline, hierarchy);

			for(auto& e : res) {
				for(auto& s : e.Shaders) {
					for(auto& t : s) {
						for(uint32 i = 0; i < parent->scopes; ++i) {
							t.Scopes.Insert(i, parent->scopes[i]);
						}
					}
				}

				result1s.EmplaceBack(GTSL::MoveRef(e));
			}

			hierarchy.EmplaceBack(parent);

			for (const auto& e : parent->Children) {
				self(e, self);
			}
		};

		call(permutation_manager, call);

		return result1s;
	}

	template<class A>
	PermutationManager* CreateChild(const GTSL::StringView name) {
		return Children.EmplaceBack(GTSL::SmartPointer<A, BE::TAR>(GetTransientAllocator(), name));
	}

	GTSL::StaticVector<GTSL::SmartPointer<PermutationManager, BE::TAR>, 8> Children;
	GTSL::StaticString<64> InstanceName;
	const GTSL::StaticString<64> ClassName;

	template<typename T>
	static const T* Find(const GTSL::StringView class_name, const GTSL::Range<const PermutationManager**> hierarchy) {
		for (const auto e : hierarchy) {
			if (e->ClassName == class_name) { //pseudo dynamic cast
				return static_cast<const T*>(e);
			}
		}

		return nullptr;
	}

	GTSL::Range<const ShaderTag*> GetTagList() { return tags; }

	static auto ProcessShaders(PermutationManager* start, GPipeline* pipeline, GTSL::JSONMember shader_group_json, GTSL::JSONMember shader_json) {
		GTSL::StaticVector<ShaderPermutation, 8> batches;
		GTSL::StaticVector<const PermutationManager*, 16> scopes;

		auto domain = GTSL::ShortString<32>(shader_group_json[u8"domain"]);

		//there's an unintended side effect in this code, where the first permutation is called regardless of whether it can process the current domain or not
		auto call = [&](PermutationManager* parent, auto&& self) -> void {
			auto res = parent->supportedDomains.Find(domain);

			if (auto l = parent->supportedDomainsFunctions[res.Get()]) {
				l(parent, pipeline, shader_group_json, shader_json, scopes, batches);
			} else {
				parent->ProcessShader(pipeline, shader_group_json, shader_json, scopes, batches);				
			}

			scopes.EmplaceBack(parent);

			for(const auto& e : parent->Children) {
				if (Contains(domain, static_cast<const PermutationManager*>(e.GetData())->supportedDomains.GetRange())) {
					self(e, self);
				}
			}

			scopes.PopBack();
		};

		call(start, call);

		if (shader_json[u8"tags"]) {
			for (auto& b : batches) {
				for (auto t : shader_json[u8"tags"]) {
					b.Tags.EmplaceBack(t[u8"name"].GetStringView(), t[u8"text"].GetStringView());
				}
			}
		}

		return batches;
	}

protected:
	using SIG = GTSL::FunctionPointer<void(GPipeline* pipeline, GTSL::JSONMember shader_group_json, GTSL::JSONMember shader_json, GTSL::Range<const PermutationManager**> hierarchy, GTSL::StaticVector<ShaderPermutation, 8>& batches)>;

	void AddTag(const GTSL::StringView name, const GTSL::StringView tag_string) { tags.EmplaceBack(name, tag_string); }

	void AddSupportedDomain(const GTSL::StringView domain_name) {
		supportedDomains.EmplaceBack(domain_name);
		supportedDomainsFunctions.EmplaceBack();
	}

	template<typename T, void(T::*L)(GPipeline*, GTSL::JSONMember, GTSL::JSONMember, GTSL::Range<const PermutationManager**>, GTSL::StaticVector<ShaderPermutation, 8>&)>
	void AddSupportedDomain(const GTSL::StringView domain_name) {
		supportedDomains.EmplaceBack(domain_name);
		supportedDomainsFunctions.EmplaceBack(SIG::Create<T, L>());
	}

	void AddScope(const GPipeline::ElementHandle scope_handle) {
		scopes.EmplaceBack(scope_handle);
	}

private:
	GTSL::StaticVector<ShaderTag, 4> tags;
	GTSL::StaticVector<GTSL::ShortString<32>, 8> supportedDomains;
	GTSL::StaticVector<SIG, 8> supportedDomainsFunctions;
	GTSL::StaticVector<GPipeline::ElementHandle, 8> scopes;
};