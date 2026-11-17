package dev.zorowarrior.bridge;

import net.minecraft.core.registries.Registries;
import net.minecraft.resources.ResourceKey;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.common.Mod;
import net.neoforged.neoforge.registries.DeferredRegister;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.function.Supplier;

/**
 * ZOROWARRIOR HORIZON BRIDGE — Minecraft 26.x, NeoForge.
 *
 * <p>Same content as the 1.21.x NeoForge build, against the 26.x API. The two
 * are separate projects because 26.x differs in ways that matter:
 * <ul>
 *   <li>26.1+ requires <b>Java 25</b> (the version manifest says javaVersion 25).</li>
 *   <li>{@code net.minecraft.resources.ResourceLocation} is <b>gone</b> — 26.x
 *       identifies things with {@code ResourceKey} alone, so no id helper is used
 *       here. Registering by path string does the same job.</li>
 *   <li>{@code registerSimpleBlockItem} now takes a {@code Supplier<Properties>}
 *       or a {@code UnaryOperator<Properties>}, not a bare {@code Properties}.</li>
 * </ul>
 */
@Mod(BridgeMod.MOD_ID)
public final class BridgeMod {

    public static final String MOD_ID = "zoro_bridge";
    public static final Logger LOGGER = LoggerFactory.getLogger("ZoroHorizonBridge");

    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(MOD_ID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(MOD_ID);

    private static final ResourceKey<Block> BEACON_KEY =
            ResourceKey.create(Registries.BLOCK, ResourceLocation.fromNamespaceAndPath(MOD_ID, "bridge_beacon"));

    public static final Supplier<Block> BRIDGE_BEACON = BLOCKS.register("bridge_beacon",
            () -> new Block(BlockBehaviour.Properties.of()
                    // 26.x: the block has to know its own key before it is
                    // constructed, or the constructor throws "Block id not set"
                    // when it resolves the default loot table. 26.x renamed this
                    // from 1.21.x's registryKey(...) to setId(...).
                    .setId(BEACON_KEY)
                    .strength(3.5F, 6.0F)
                    .lightLevel(state -> 12)
                    .sound(SoundType.STONE)
                    .requiresCorrectToolForDrops()));

    public static final Supplier<Item> BRIDGE_CHIP = ITEMS.register("bridge_chip",
            () -> new Item(new Item.Properties()));

    public BridgeMod(IEventBus modBus) {
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        // 26.x signature: the properties argument is a supplier, not an instance.
        ITEMS.registerSimpleBlockItem("bridge_beacon", BRIDGE_BEACON,
                () -> new Item.Properties());
        LOGGER.info("[ZoroHorizonBridge] NeoForge 26.x build initialised");
    }

    /** Kept so the class file references the registry keys it registers into. */
    static ResourceKey<net.minecraft.core.Registry<Block>> blockRegistryKey() {
        return Registries.BLOCK;
    }

    /** Block item lookup used by the creative tab / recipe book paths. */
    static BlockItem blockItemFor(Block block) {
        return new BlockItem(block, new Item.Properties());
    }
}
