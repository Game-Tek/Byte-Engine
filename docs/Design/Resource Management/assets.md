Assets are loaded in the following manner:
- `get()` is called by a user on the resource manager.
- The resource manager checks if the resource is already cached/processed.
	- If it is, the resource document is fetched along with it's dependencies which are resolved recursivelly. In the end all resources will be provided to the user is such a manner that they are loaded in the correct order, that is all children will be loaded before their parents.
	- If it is not, the resource manager will invoke all resource handlers so that they can handle the asset if needed.