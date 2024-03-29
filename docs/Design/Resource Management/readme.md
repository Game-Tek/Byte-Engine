This section details who the asset processing and resource loading is approached in the engine. From the asset loading during development to the resource handling during runtime.

Some terminology to keep in mind:
* **Asset**: User provided file describing something that you want to consume in your application. (JPEG, PNG, GLTF, JSON, etc)
* **Resource**: An asset that has been processed by the engine. Contains useful metadata such as binary size, format, hash and the actual resource info like texture extent, mesh vertex description, etc. Resources are usually comprised of both binary data and structured information such as a document where the metadata is stored.
* **URL**: A URL is a reference to an asset or resource. The asset extension must not be named. The URL can be a filesystem path relative to the /assets folder or an internet URL.