#pragma once

struct PermutationManager : Object {
	struct ShaderGenerationData {
		GTSL::StaticVector<const PermutationManager*, 16> Hierarchy;
	};

	using ShaderTag = GTSL::Pair<GTSL::ShortString<32>, GTSL::ShortString<32>>;

	PermutationManager(const GTSL::StringView instance_name, const GTSL::StringView class_name) : InstanceName(instance_name), ClassName(class_name), JSON(GetPersistentAllocator()) {

	}

	static GTSL::StaticString<8192> MakeShaderString(GTSL::StringView raw_code, GTSL::StringView user_shader_code = {}) {
		GTSL::StaticString<8192> shaderCode;

		auto i = raw_code.begin();

		while(true) {
			auto s = i;

			while(i != raw_code.end() && *i != u8'@') {
				++i;
			}

			shaderCode += { s, i };

			if(i == raw_code.end()) {
				break;
			}

			shaderCode += user_shader_code;

			++i;

			while(IsAnyOf(*i, u8'\n', u8'\f', u8'\r')) {
				++i;
			}
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

	template<class A>
	PermutationManager* CreateChild(const GTSL::StringView name) {
		auto permutationManager = GTSL::SmartPointer<A, BE::PAR>(GetPersistentAllocator(), name);
		auto* permutationManagerPtr = permutationManager.GetData();
		Children.EmplaceBack(GTSL::MoveRef(permutationManager));
		return static_cast<PermutationManager*>(permutationManagerPtr);
	}

	GTSL::StaticVector<GTSL::SmartPointer<PermutationManager, BE::PAR>, 8> Children;
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

	void AddSupportedDomain(const GTSL::StringView domain_name) {
		if(supportedDomains.Find(domain_name)) { return; }
		supportedDomains.EmplaceBack(domain_name);
	}

	auto& GetSupportedDomains() const {return supportedDomains;}
	
	GTSL::JSON<BE::PAR> JSON;

	GTSL::StaticVector<GTSL::Vector<GTSL::Buffer<BE::PAR>, BE::PAR>, 3> a;

	auto GetChildren() -> GTSL::StaticVector<PermutationManager*, 8> {
		GTSL::StaticVector<PermutationManager*, 8> children;
		for (auto& e : Children) {
			children.EmplaceBack(e.GetData());
		}
		return children;
	}

protected:
	using SIG = GTSL::FunctionPointer<void(GPipeline* pipeline, const GTSL::JSON<BE::PAR>&, const GTSL::JSON<BE::PAR>&, GTSL::Range<const PermutationManager**> hierarchy, GTSL::StaticVector<ShaderPermutation, 8>& batches)>;

	void AddTag(const GTSL::StringView name, const GTSL::StringView tag_string) { tags.EmplaceBack(name, tag_string); }

private:
	GTSL::StaticVector<ShaderTag, 4> tags;
	GTSL::StaticVector<GTSL::StaticString<32>, 4> supportedDomains;
};