## Purpose

​		The account module primarily supports users to use Buckyos with the traditional username and password method, optimizing the user experience of Buckyos.

​		Since the underlying implementation of Buckyos uses the [W3C DID](https://www.w3.org/TR/2022/REC-did-core-20220719/) specification, which relies on technologies such as public-private key encryption and blockchain, directly using DID would require users to understand the associated technologies. This high requirement on users would greatly hinder the promotion of Buckyos. Therefore, a familiar method for users is needed, and username and password is a method that all internet users have accepted and used before.

## Account Types

Buckyos accounts come in two types: centralized accounts and local accounts.

##### Centralized Account

- The account service provided by Buckyos is centralized, with user information stored on servers provided by Buckyos. DID can directly use the username, making it more natural for users and providing a better experience.


- Supports changing the login password, but the login account cannot be changed.
- Supports modifying the key from the cloud.
- Accounts can be migrated from the server to personal devices, but the migration process requires the relevant information to be put on the blockchain, which incurs a certain cost for the user.
- Must be actively registered by the user.
- User permissions belong to Zones. Different Zones have different user permissions. Zone administrators can invite other users to become users of that Zone and specify their permissions within the Zone.
- Newly registered accounts do not belong to any Zone. If a new Zone is created, the user automatically becomes the administrator of that Zone, or they can become users of other Zones through invitations.
- Only Zone administrators can bind devices. After binding, local accounts with the same username and password but different DID information as the centralized account will be generated on the device. If the local Zone is online, it uses the centralized account's DID; if offline, it uses the local DID information. After logging in, users can perform all operations of the local account based on their permissions.
- DID format: did:bid:$username

##### Local Account

- Users create accounts on their local Zone, with user information stored on the devices contained within the Zone. Local accounts can only have self-contained DIDs, meaning the validity of the DID can be verified by the DID Document itself without needing to download the DID Document from a trusted source, such as when the DID is the hash of the public key.


- When a remote account binds a device, a local account with the same username and password as the remote account is generated locally.
- Supports changing the login password, but the login account cannot be changed.
- Does not support modifying the key.
- User permissions belong to Zones. Different Zones have different user permissions. Zone administrators can directly create local accounts, delete accounts, and invite other users to become users of that Zone. The management page allows modifying user permissions within the Zone.
- The first account created in a local Zone automatically becomes the administrator of that Zone.
- Administrators of local accounts can also adjust user permissions within the Zone.
- DID format: did:lbid:$publickey_hash

Regardless of whether it is a centralized or local account, once logged in, DID-related information is generated, and the user's subsequent operations follow the DID permission verification logic.

## Basic Account Information

Username, password, DID, DID Document.

## Implementation Description

Based on the above description, the core logic of centralized and local accounts is the same, except for the storage method. Therefore, it can be implemented in parts:

1. Private Key Management Module: Responsible for managing private keys, including private key creation, storage, and signing operations.
2. Account Storage Module: Mainly implements the storage of accounts, differing between centralized and local accounts.
3. Basic Account Module: Includes interfaces for account creation, management, and login verification. Upper-layer services use account services through this module's interfaces. By parameterizing the account management module and private key management module, the different needs of centralized and local accounts can be met.
4. Business Requirement Module: Calls the interfaces of the basic account module to meet business needs, such as supporting cross-process and cross-network interfaces, and local account administrator account creation and deletion.
