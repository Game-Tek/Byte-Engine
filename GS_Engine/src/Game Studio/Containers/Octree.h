#pragma once

#include "Core.h"

#include "FVector.hpp"

template <class T>
struct GS_API OctreeNode
{
	OctreeNode();

	explicit OctreeNode(OctreeNode * Parent);

	//Deletes this node instance and all of it's children.
	//Remember the deletion will propagate down the branch, effectively "killing" all the branches and nodes below it.
	~OctreeNode();

	//Fills all of the branches with newly allocated nodes.
	void CreateChildren();

	//Deletes all of the node's children.
	void KillChildren();

	//Unlinks the parent and the child of the selected branch, but keeps it in memory.
	void PruneBranch(uint8 ChildIndex);

	//Deletes all children found under the branch to be killed. Destroys them.
	void KillBranch(uint8 ChildIndex);

	//Sets the node's parent as the NewParent.
	void ReParent(OctreeNode * NewParent);

	//Returns a pointer to this node's parent.
	OctreeNode<T> * GetParent() { return Parent; }

	//Returns a pointer to the array of children.
	OctreeNode<T> * GetChildren() { return Children; }

	//Returns a pointer to the specified child.
	OctreeNode<T> * GetChild(uint8 ChildIndex) { return Children[ChildIndex]; }

	//Returns a reference to the FVector holding all of this node's elements.
	FVector<T> & GetElements() { return Elements; }

	//Returns a reference to the specified child node.
	OctreeNode<T> & operator[](uint8 Index) { return (*Children[Index]); }

private:
	//Pointer to this node's parent.
	OctreeNode<T> * Parent = nullptr;

	//Array of pointers to this node's eight children.
	OctreeNode * Children[8];

	//Elements contained in this node.
	FVector<T> Elements;
};

template <class T>
OctreeNode<T>::OctreeNode() : Elements(8)
{
}

template <class T>
OctreeNode<T>::OctreeNode(OctreeNode * Parent) : Parent(Parent), Elements(8)
{
}

template <class T>
void OctreeNode<T>::ReParent(OctreeNode * NewParent)
{
	this->Parent = NewParent;

	return;
}

template <class T>
void OctreeNode<T>::CreateChildren()
{
	this->Children[0] = new OctreeNode(this);
	this->Children[1] = new OctreeNode(this);
	this->Children[2] = new OctreeNode(this);
	this->Children[3] = new OctreeNode(this);
	this->Children[4] = new OctreeNode(this);
	this->Children[5] = new OctreeNode(this);
	this->Children[6] = new OctreeNode(this);
	this->Children[7] = new OctreeNode(this);

	return;
}

template <class T>
void OctreeNode<T>::KillChildren()
{
	delete this->Children[0];
	delete this->Children[1];
	delete this->Children[2];
	delete this->Children[3];
	delete this->Children[4];
	delete this->Children[5];
	delete this->Children[6];
	delete this->Children[7];

	this->Children[0] = nullptr;
	this->Children[1] = nullptr;
	this->Children[2] = nullptr;
	this->Children[3] = nullptr;
	this->Children[4] = nullptr;
	this->Children[5] = nullptr;
	this->Children[6] = nullptr;
	this->Children[7] = nullptr;

	return;
}

template <class T>
void OctreeNode<T>::PruneBranch(uint8 ChildIndex)
{
	this->Children[ChildIndex]->Parent = nullptr;

	this->Children[ChildIndex] = nullptr;

	return;
}

template <class T>
void OctreeNode<T>::KillBranch(uint8 ChildIndex)
{
	delete this->Children[ChildIndex];

	this->Children[ChildIndex] = nullptr;

	return;
}

template <class T>
OctreeNode<T>::~OctreeNode()
{
	delete this->Children[0];
	delete this->Children[1];
	delete this->Children[2];
	delete this->Children[3];
	delete this->Children[4];
	delete this->Children[5];
	delete this->Children[6];
	delete this->Children[7];
}

template <class T>
class GS_API Octree
{
public:
	Octree();

private:
	//This octree's root node.
	OctreeNode<T> RootNode;
};
